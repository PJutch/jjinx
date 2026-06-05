use crate::parse::Request;
use std::{collections::HashMap, convert::Infallible, env};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VarError {
    #[error("Unknow variable {0}")]
    UnknownVar(String),
}

fn to_pascal_case(str: &str) -> String {
    let mut result = String::new();
    let mut prev_letter = false;
    for c in str.chars() {
        if c.is_ascii_lowercase() && !prev_letter {
            result.push(c.to_ascii_uppercase());
        } else if c == '_' {
            result.push('-');
        } else {
            result.push(c);
        }

        prev_letter = c.is_ascii_alphabetic();
    }
    result
}

fn dynamic_replacer(var: &str, output: &mut String, request: &Request) -> Result<(), VarError> {
    if let Some(header) = var.strip_prefix("http_") {
        let header = to_pascal_case(header);
        if let Some(value) = request.headers.get(&header) {
            output.push_str(value);
            return Ok(());
        }
    }

    if let Some(env) = var.strip_prefix("env_") {
        let env = env.to_ascii_uppercase();
        if let Ok(data) = env::var(env) {
            output.push_str(&data);
            return Ok(());
        }
    }

    match var {
        "uri" => output.push_str(&request.start_line.uri.full),
        "host" => {
            if request.start_line.uri.host.is_empty() {
                output.push_str(
                    &request
                        .headers
                        .get("Host")
                        .map(|s| s.as_str())
                        .unwrap_or(""),
                );
            } else {
                output.push_str(&request.start_line.uri.host)
            }
        }
        "method" => output.push_str(&request.start_line.uri.host),
        _ => return Err(VarError::UnknownVar(var.to_owned())),
    }
    Ok(())
}

fn replace_vars<Err, F: Fn(&str, &mut String) -> Result<(), Err>>(
    str: &str,
    replacer: F,
) -> Result<String, Err> {
    let mut result = String::new();

    let mut is_var = false;
    let mut last_var = String::new();

    for (_, c) in str.chars().enumerate() {
        if is_var {
            if c.is_ascii_alphanumeric() || c == '_' {
                last_var.push(c);
                continue;
            } else {
                replacer(&last_var, &mut result)?;

                is_var = false;
                last_var.clear();
            }
        }

        if c == '$' {
            is_var = true;
        } else {
            result.push(c);
        }
    }

    if is_var {
        replacer(&last_var, &mut result)?;
    }

    Ok(result)
}

pub fn replace_dynamic_vars(str: &str, request: &Request) -> Result<String, VarError> {
    replace_vars(str, |var, output| dynamic_replacer(var, output, request))
}

fn static_replacer(
    var: &str,
    output: &mut String,
    vars: &[&HashMap<String, String>],
) -> Result<(), Infallible> {
    for var_set in vars {
        if let Some(value) = var_set.get(var) {
            output.push_str(value);
            return Ok(());
        }
    }

    output.push('$');
    output.push_str(var);
    Ok(())
}

pub fn replace_static_vars(
    str: &str,
    vars: &[&HashMap<String, String>],
) -> Result<String, Infallible> {
    replace_vars(str, |var, output| static_replacer(var, output, vars))
}
