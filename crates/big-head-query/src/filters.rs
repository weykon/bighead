//! Token boundaries for presence filters; quoted literals remain untouched.
use crate::QueryError;

pub(crate) fn split_filter_tokens(input: &str) -> Result<Vec<&str>, QueryError> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut brackets = Vec::new();
    for (offset, ch) in input.char_indices() {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == delimiter {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                continue;
            }
            '(' | '[' | '{' => {
                brackets.push(ch);
                continue;
            }
            ')' | ']' | '}' => {
                let expected = match ch {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                if brackets.pop() != Some(expected) {
                    return Err(QueryError::ParseError(
                        "Unbalanced filter delimiters".into(),
                    ));
                }
                continue;
            }
            _ => {}
        }
        if !brackets.is_empty() {
            continue;
        }
        let and = input
            .get(offset..offset + 3)
            .is_some_and(|s| s.eq_ignore_ascii_case("and"))
            && input[..offset]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
            && input[offset + 3..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
        if ch == ',' || and {
            let token = input[start..offset].trim();
            if token.is_empty() {
                return Err(QueryError::ParseError("Empty filter".into()));
            }
            tokens.push(token);
            start = offset + if and { 3 } else { 1 };
        }
    }
    if quote.is_some() || !brackets.is_empty() {
        return Err(QueryError::ParseError(
            "Unclosed filter literal or parentheses".into(),
        ));
    }
    let tail = input[start..].trim();
    if tail.is_empty() {
        return Err(QueryError::ParseError("Empty filter".into()));
    }
    tokens.push(tail);
    Ok(tokens)
}

pub(crate) fn parse_with_without_keyword(
    token: &str,
) -> Result<Option<(bool, Vec<&str>)>, QueryError> {
    let Some((keyword, tail)) = token.split_once('(') else {
        return Ok(None);
    };
    let keyword = keyword.trim();
    let negated = if keyword.eq_ignore_ascii_case("without") {
        true
    } else if keyword.eq_ignore_ascii_case("with") {
        false
    } else {
        return Ok(None);
    };
    let body = tail.strip_suffix(')').ok_or_else(|| {
        QueryError::ParseError("WITH/WITHOUT requires closing parentheses".into())
    })?;
    let names: Vec<_> = body.split(',').map(str::trim).collect();
    if names.iter().any(|s| {
        s.is_empty()
            || s.chars()
                .any(|c| matches!(c, '(' | ')' | '\'' | '"') || c.is_whitespace())
    }) {
        return Err(QueryError::ParseError(
            "WITH/WITHOUT expects component names".into(),
        ));
    }
    Ok(Some((negated, names)))
}
