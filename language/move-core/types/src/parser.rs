// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    account_address::AccountAddress,
    identifier::{self, Identifier},
    language_storage::{StructTag, TypeTag},
    transaction_argument::TransactionArgument,
};
use anyhow::{bail, format_err, Result};
use std::iter::Peekable;

#[derive(Eq, PartialEq, Debug)]
enum Token {
    U8Type,
    U64Type,
    U128Type,
    BoolType,
    AddressType,
    VectorType,
    SignerType,
    Whitespace(String),
    Name(String),
    Address(String),
    U8(String),
    U64(String),
    U128(String),
    Bytes(String),
    True,
    False,
    ColonColon,
    Lt,
    Gt,
    Comma,
    EOF,
}

impl Token {
    fn is_whitespace(&self) -> bool {
        matches!(self, Self::Whitespace(_))
    }
}

fn name_token(s: String) -> Token {
    match s.as_str() {
        "u8" => Token::U8Type,
        "u64" => Token::U64Type,
        "u128" => Token::U128Type,
        "bool" => Token::BoolType,
        "address" => Token::AddressType,
        "vector" => Token::VectorType,
        "true" => Token::True,
        "false" => Token::False,
        "signer" => Token::SignerType,
        _ => Token::Name(s),
    }
}

fn next_number(initial: char, mut it: impl Iterator<Item = char>) -> Result<(Token, usize)> {
    let mut num = String::new();
    num.push(initial);
    loop {
        match it.next() {
            Some(c) if c.is_ascii_digit() => num.push(c),
            Some(c) if c.is_alphanumeric() => {
                let mut suffix = String::new();
                suffix.push(c);
                loop {
                    match it.next() {
                        Some(c) if c.is_ascii_alphanumeric() => suffix.push(c),
                        _ => {
                            let len = num.len() + suffix.len();
                            let tok = match suffix.as_str() {
                                "u8" => Token::U8(num),
                                "u64" => Token::U64(num),
                                "u128" => Token::U128(num),
                                _ => bail!("invalid suffix"),
                            };
                            return Ok((tok, len));
                        }
                    }
                }
            }
            _ => {
                let len = num.len();
                return Ok((Token::U64(num), len));
            }
        }
    }
}

#[allow(clippy::many_single_char_names)]
fn next_token(s: &str) -> Result<Option<(Token, usize)>> {
    let mut it = s.chars().peekable();
    match it.next() {
        None => Ok(None),
        Some(c) => Ok(Some(match c {
            '<' => (Token::Lt, 1),
            '>' => (Token::Gt, 1),
            ',' => (Token::Comma, 1),
            ':' => match it.next() {
                Some(':') => (Token::ColonColon, 2),
                _ => bail!("unrecognized token"),
            },
            '0' if it.peek() == Some(&'x') || it.peek() == Some(&'X') => {
                it.next().unwrap();
                match it.next() {
                    Some(c) if c.is_ascii_hexdigit() => {
                        let mut r = String::new();
                        r.push('0');
                        r.push('x');
                        r.push(c);
                        for c in it {
                            if c.is_ascii_hexdigit() {
                                r.push(c);
                            } else {
                                break;
                            }
                        }
                        let len = r.len();
                        (Token::Address(r), len)
                    }
                    _ => bail!("unrecognized token"),
                }
            }
            c if c.is_ascii_digit() => next_number(c, it)?,
            'b' if it.peek() == Some(&'"') => {
                it.next().unwrap();
                let mut r = String::new();
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some(c) if c.is_ascii() => r.push(c),
                        _ => bail!("unrecognized token"),
                    }
                }
                let len = r.len() + 3;
                (Token::Bytes(hex::encode(r)), len)
            }
            'x' if it.peek() == Some(&'"') => {
                it.next().unwrap();
                let mut r = String::new();
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some(c) if c.is_ascii_hexdigit() => r.push(c),
                        _ => bail!("unrecognized token"),
                    }
                }
                let len = r.len() + 3;
                (Token::Bytes(r), len)
            }
            c if c.is_ascii_whitespace() => {
                let mut r = String::new();
                r.push(c);
                for c in it {
                    if c.is_ascii_whitespace() {
                        r.push(c);
                    } else {
                        break;
                    }
                }
                let len = r.len();
                (Token::Whitespace(r), len)
            }
            c if c.is_ascii_alphabetic() => {
                let mut r = String::new();
                r.push(c);
                for c in it {
                    if identifier::is_valid_identifier_char(c) {
                        r.push(c);
                    } else {
                        break;
                    }
                }
                let len = r.len();
                (name_token(r), len)
            }
            _ => bail!("unrecognized token"),
        })),
    }
}

fn tokenize(mut s: &str) -> Result<Vec<Token>> {
    let mut v = vec![];
    while let Some((tok, n)) = next_token(s)? {
        v.push(tok);
        s = &s[n..];
    }
    Ok(v)
}

struct Parser<I: Iterator<Item = Token>> {
    it: Peekable<I>,
}

impl<I: Iterator<Item = Token>> Parser<I> {
    fn new<T: IntoIterator<Item = Token, IntoIter = I>>(v: T) -> Self {
        Self {
            it: v.into_iter().peekable(),
        }
    }

    fn next(&mut self) -> Result<Token> {
        match self.it.next() {
            Some(tok) => Ok(tok),
            None => bail!("out of tokens, this should not happen"),
        }
    }

    fn peek(&mut self) -> Option<&Token> {
        self.it.peek()
    }

    fn consume(&mut self, tok: Token) -> Result<()> {
        let t = self.next()?;
        if t != tok {
            bail!("expected token {:?}, got {:?}", tok, t)
        }
        Ok(())
    }

    fn parse_comma_list<F, R>(
        &mut self,
        parse_list_item: F,
        end_token: Token,
        allow_trailing_comma: bool,
    ) -> Result<Vec<R>>
    where
        F: Fn(&mut Self) -> Result<R>,
        R: std::fmt::Debug,
    {
        let mut v = vec![];
        if !(self.peek() == Some(&end_token)) {
            loop {
                v.push(parse_list_item(self)?);
                if self.peek() == Some(&end_token) {
                    break;
                }
                self.consume(Token::Comma)?;
                if self.peek() == Some(&end_token) && allow_trailing_comma {
                    break;
                }
            }
        }
        Ok(v)
    }

    fn parse_string(&mut self) -> Result<String> {
        Ok(match self.next()? {
            Token::Name(s) => s,
            tok => bail!("unexpected token {:?}, expected string", tok),
        })
    }

    fn parse_type_tag(&mut self, depth: u8) -> Result<TypeTag> {
        if depth >= crate::safe_serialize::MAX_TYPE_TAG_NESTING {
            bail!("Exceeded TypeTag nesting limit during parsing: {}", depth);
        }
        Ok(match self.next()? {
            Token::U8Type => TypeTag::U8,
            Token::U64Type => TypeTag::U64,
            Token::U128Type => TypeTag::U128,
            Token::BoolType => TypeTag::Bool,
            Token::AddressType => TypeTag::Address,
            Token::SignerType => TypeTag::Signer,
            Token::VectorType => {
                self.consume(Token::Lt)?;
                let ty = self.parse_type_tag(depth + 1)?;
                self.consume(Token::Gt)?;
                TypeTag::Vector(Box::new(ty))
            }
            Token::Address(addr) => {
                self.consume(Token::ColonColon)?;
                match self.next()? {
                    Token::Name(module) => {
                        self.consume(Token::ColonColon)?;
                        match self.next()? {
                            Token::Name(name) => {
                                let ty_args = if self.peek() == Some(&Token::Lt) {
                                    self.next()?;
                                    let ty_args = self.parse_comma_list(
                                        |parser| parser.parse_type_tag(depth + 1),
                                        Token::Gt,
                                        true,
                                    )?;
                                    self.consume(Token::Gt)?;
                                    ty_args
                                } else {
                                    vec![]
                                };
                                TypeTag::Struct(Box::new(StructTag {
                                    address: AccountAddress::from_hex_literal(&addr)?,
                                    module: Identifier::new(module)?,
                                    name: Identifier::new(name)?,
                                    type_params: ty_args,
                                }))
                            }
                            t => bail!("expected name, got {:?}", t),
                        }
                    }
                    t => bail!("expected name, got {:?}", t),
                }
            }
            tok => bail!("unexpected token {:?}, expected type tag", tok),
        })
    }

    fn parse_transaction_argument(&mut self) -> Result<TransactionArgument> {
        Ok(match self.next()? {
            Token::U8(s) => TransactionArgument::U8(s.parse()?),
            Token::U64(s) => TransactionArgument::U64(s.parse()?),
            Token::U128(s) => TransactionArgument::U128(s.parse()?),
            Token::True => TransactionArgument::Bool(true),
            Token::False => TransactionArgument::Bool(false),
            Token::Address(addr) => {
                TransactionArgument::Address(AccountAddress::from_hex_literal(&addr)?)
            }
            Token::Bytes(s) => TransactionArgument::U8Vector(hex::decode(s)?),
            tok => bail!("unexpected token {:?}, expected transaction argument", tok),
        })
    }
}

fn parse<F, T>(s: &str, f: F) -> Result<T>
where
    F: Fn(&mut Parser<std::vec::IntoIter<Token>>) -> Result<T>,
{
    let mut tokens: Vec<_> = tokenize(s)?
        .into_iter()
        .filter(|tok| !tok.is_whitespace())
        .collect();
    tokens.push(Token::EOF);
    let mut parser = Parser::new(tokens);
    let res = f(&mut parser)?;
    parser.consume(Token::EOF)?;
    Ok(res)
}

pub fn parse_string_list(s: &str) -> Result<Vec<String>> {
    parse(s, |parser| {
        parser.parse_comma_list(|parser| parser.parse_string(), Token::EOF, true)
    })
}

pub fn parse_type_tags(s: &str) -> Result<Vec<TypeTag>> {
    parse(s, |parser| {
        parser.parse_comma_list(|parser| parser.parse_type_tag(0), Token::EOF, true)
    })
}

pub fn parse_type_tag(s: &str) -> Result<TypeTag> {
    parse(s, |parser| parser.parse_type_tag(0))
}

pub fn parse_transaction_arguments(s: &str) -> Result<Vec<TransactionArgument>> {
    parse(s, |parser| {
        parser.parse_comma_list(
            |parser| parser.parse_transaction_argument(),
            Token::EOF,
            true,
        )
    })
}

pub fn parse_transaction_argument(s: &str) -> Result<TransactionArgument> {
    parse(s, |parser| parser.parse_transaction_argument())
}

pub fn parse_struct_tag(s: &str) -> Result<StructTag> {
    let type_tag = parse(s, |parser| parser.parse_type_tag(0))
        .map_err(|e| format_err!("invalid struct tag: {}, {}", s, e))?;
    if let TypeTag::Struct(struct_tag) = type_tag {
        Ok(*struct_tag)
    } else {
        bail!("invalid struct tag: {}", s)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        account_address::AccountAddress,
        parser::{parse_struct_tag, parse_transaction_argument, parse_type_tag},
        transaction_argument::TransactionArgument,
    };

    #[allow(clippy::unreadable_literal)]
    #[test]
    fn tests_parse_transaction_argument_positive() {
        use TransactionArgument as T;

        for (s, expected) in &[
            ("  0u8", T::U8(0)),
            ("0u8", T::U8(0)),
            ("255u8", T::U8(255)),
            ("0", T::U64(0)),
            ("0123", T::U64(123)),
            ("0u64", T::U64(0)),
            ("18446744073709551615", T::U64(18446744073709551615)),
            ("18446744073709551615u64", T::U64(18446744073709551615)),
            ("0u128", T::U128(0)),
            (
                "340282366920938463463374607431768211455u128",
                T::U128(340282366920938463463374607431768211455),
            ),
            ("true", T::Bool(true)),
            ("false", T::Bool(false)),
            (
                "0x0",
                T::Address(AccountAddress::from_hex_literal("0x0").unwrap()),
            ),
            (
                "0x54afa3526",
                T::Address(AccountAddress::from_hex_literal("0x54afa3526").unwrap()),
            ),
            (
                "0X54afa3526",
                T::Address(AccountAddress::from_hex_literal("0x54afa3526").unwrap()),
            ),
            ("x\"7fff\"", T::U8Vector(vec![0x7f, 0xff])),
            ("x\"\"", T::U8Vector(vec![])),
            ("x\"00\"", T::U8Vector(vec![0x00])),
            ("x\"deadbeef\"", T::U8Vector(vec![0xde, 0xad, 0xbe, 0xef])),
        ] {
            assert_eq!(&parse_transaction_argument(s).unwrap(), expected)
        }
    }

    #[test]
    fn tests_parse_transaction_argument_negative() {
        for s in &[
            "-3",
            "0u42",
            "0u645",
            "0u64x",
            "0u6 4",
            "0u",
            "256u8",
            "18446744073709551616",
            "18446744073709551616u64",
            "340282366920938463463374607431768211456u128",
            "0xg",
            "0x00g0",
            "0x",
            "0x_",
            "",
            "x\"ffff",
            "x\"a \"",
            "x\" \"",
            "x\"0g\"",
            "x\"0\"",
            "garbage",
            "true3",
            "3false",
            "3 false",
            "",
        ] {
            assert!(parse_transaction_argument(s).is_err())
        }
    }

    #[test]
    fn test_type_tag() {
        for s in &[
            "u64",
            "bool",
            "vector<u8>",
            "vector<vector<u64>>",
            "vector<vector<vector<vector<vector<vector<vector<u64>>>>>>>",
            "signer",
            "0x1::M::S",
            "0x2::M::S_",
            "0x3::M_::S",
            "0x4::M_::S_",
            "0x00000000004::M::S",
            "0x1::M::S<u64>",
            "0x1::M::S<0x2::P::Q>",
            "vector<0x1::M::S>",
            "vector<0x1::M_::S_>",
            "vector<vector<0x1::M_::S_>>",
            "0x1::M::S<vector<u8>>",
        ] {
            assert!(parse_type_tag(s).is_ok(), "Failed to parse tag {}", s);
        }

        let s =
            "vector<vector<vector<vector<vector<vector<vector<vector<vector<vector<vector<vector<vector<vector<u64>>>>>>>>>>>>>>";
        assert!(
            parse_type_tag(s).is_err(),
            "Should have failed to parse type tag {}",
            s
        );
    }

    #[test]
    fn test_parse_valid_struct_tag() {
        let valid = vec![
            "0x1::Diem::Diem",
            "0x1::Diem_Type::Diem",
            "0x1::Diem_::Diem",
            "0x1::X_123::X32_",
            "0x1::Diem::Diem_Type",
            "0x1::Diem::Diem<0x1::XDX::XDX>",
            "0x1::Diem::Diem<0x1::XDX::XDX_Type>",
            "0x1::Diem::Diem<u8>",
            "0x1::Diem::Diem<u64>",
            "0x1::Diem::Diem<u128>",
            "0x1::Diem::Diem<bool>",
            "0x1::Diem::Diem<address>",
            "0x1::Diem::Diem<signer>",
            "0x1::Diem::Diem<vector<0x1::XDX::XDX>>",
            "0x1::Diem::Diem<u8,bool>",
            "0x1::Diem::Diem<u8,   bool>",
            "0x1::Diem::Diem<u8  ,bool>",
            "0x1::Diem::Diem<u8 , bool  ,    vector<u8>,address,signer>",
            "0x1::Diem::Diem<vector<0x1::Diem::Struct<0x1::XUS::XUS>>>",
            "0x1::Diem::Diem<0x1::Diem::Struct<vector<0x1::XUS::XUS>, 0x1::Diem::Diem<vector<0x1::Diem::Struct<0x1::XUS::XUS>>>>>",
            "0x1::Diem::Diem<vector<vector<vector<vector<vector<0x1::XDX::XDX<vector<vector<vector<0x1::XDX::XDX<vector<u64>>>>>>>>>>>>",
        ];
        for text in valid {
            let st = parse_struct_tag(text).expect("valid StructTag");
            assert_eq!(
                st.to_string().replace(" ", ""),
                text.replace(" ", ""),
                "text: {:?}, StructTag: {:?}",
                text,
                st
            );
        }

        let s = "vector<vector<vector<vector<0x1::Diem::Diem<vector<0x1::XDX::XDX<vector<vector<vector<0x1::XDX::XDX<vector<vector<u64>>>>>>>>>>>>>";
        assert!(
            parse_struct_tag(s).is_err(),
            "Should have failed to parse type tag {}",
            s
        );        
    }
}
