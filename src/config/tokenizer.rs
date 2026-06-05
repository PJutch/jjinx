use std::{ascii::escape_default, io::BufRead};

use super::ParseError;

fn skip_whitespace<Reader: BufRead>(reader: &mut Reader) -> Result<(), ParseError> {
    loop {
        let buf = reader.fill_buf().map_err(ParseError::IoError)?;
        if buf.is_empty() {
            return Ok(());
        }

        for (i, c) in buf.iter().copied().enumerate() {
            if !c.is_ascii_whitespace() || c == b'\n' {
                reader.consume(i);
                return Ok(());
            }
        }

        let len = buf.len();
        reader.consume(len);
    }
}

#[derive(PartialEq)]
enum TokenizerMode {
    NORMAL,
    QUOTED,
    COMMENT,
    ESCAPE,
}

pub fn next_token<Reader: BufRead>(reader: &mut Reader) -> Result<String, ParseError> {
    skip_whitespace(reader)?;

    let mut token = Vec::new();
    let mut mode = TokenizerMode::NORMAL;

    'fill_buf: loop {
        let buf = reader.fill_buf().map_err(ParseError::IoError)?;
        if buf.is_empty() {
            break;
        }

        for (i, c) in buf.iter().copied().enumerate() {
            match mode {
                TokenizerMode::ESCAPE => {
                    match c {
                        b'n' => {
                            token.push(b'\n');
                        }
                        b'r' => {
                            token.push(b'\r');
                        }
                        b't' => {
                            token.push(b'\t');
                        }
                        b'\\' => {
                            token.push(b'\\');
                        }
                        b'"' => {
                            token.push(b'"');
                        }
                        _ => {
                            return Err(ParseError::UnknownEscape(escape_default(c).to_string()));
                        }
                    }
                    mode = TokenizerMode::QUOTED;
                }
                TokenizerMode::QUOTED => match c {
                    b'"' => {
                        reader.consume(i + 1);
                        break 'fill_buf;
                    }
                    b'\\' => {
                        mode = TokenizerMode::ESCAPE;
                    }
                    _ => {
                        token.push(c);
                    }
                },
                TokenizerMode::COMMENT => {
                    if c == b'\n' {
                        if token.is_empty() {
                            token.push(c);
                            reader.consume(i + 1);
                        } else {
                            reader.consume(i);
                        }
                        break 'fill_buf;
                    }
                }
                TokenizerMode::NORMAL => match c {
                    b'#' => {
                        mode = TokenizerMode::COMMENT;
                    }
                    b'"' => {
                        if token.is_empty() {
                            mode = TokenizerMode::QUOTED;
                        } else {
                            reader.consume(i);
                            break 'fill_buf;
                        }
                    }
                    b'\n' if token.is_empty() => {
                        token.push(c);
                        reader.consume(i + 1);
                        break 'fill_buf;
                    }
                    _ if c.is_ascii_whitespace() => {
                        reader.consume(i);
                        break 'fill_buf;
                    }
                    b'}' if !token.is_empty() => {
                        reader.consume(i);
                        break 'fill_buf;
                    }
                    _ => {
                        token.push(c);
                    }
                },
            }
        }

        let len = buf.len();
        reader.consume(len);
    }

    String::from_utf8(token).map_err(ParseError::Utf8Error)
}

pub fn consume_fixed<Reader: BufRead>(reader: &mut Reader, expected: &str) -> Result<(), ParseError> {
    let token = next_token(reader)?;
    if token != expected {
        Err(ParseError::UnexpectedToken(token, expected.to_owned()))
    } else {
        Ok(())
    }
}

pub fn read_tokens_until_newline<Reader: BufRead>(
    reader: &mut Reader,
) -> Result<Vec<String>, ParseError> {
    let mut result = Vec::new();
    loop {
        let token = next_token(reader)?;
        if token == "\n" {
            break;
        }
        result.push(token);
    }
    Ok(result)
}
