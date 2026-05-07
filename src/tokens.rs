use once_cell::sync::Lazy;
use regex::Regex;

static TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").unwrap());

pub fn split_identifier(token: &str) -> Vec<String> {
    let lower = token.to_lowercase();
    let parts: Vec<String> = if token.contains('_') {
        lower
            .split('_')
            .filter(|p| !p.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    } else {
        split_camel(token)
    };

    if parts.iter().any(|part| part != &lower) {
        let mut out = Vec::with_capacity(parts.len() + 1);
        out.push(lower);
        out.extend(parts);
        out
    } else {
        vec![lower]
    }
}

fn split_camel(token: &str) -> Vec<String> {
    let chars: Vec<(usize, char)> = token.char_indices().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut starts = vec![0usize];
    for i in 1..chars.len() {
        let prev = chars[i - 1].1;
        let current = chars[i].1;
        let next = chars.get(i + 1).map(|(_, c)| *c);
        let boundary = (prev.is_ascii_lowercase()
            && (current.is_ascii_uppercase() || current.is_ascii_digit()))
            || (prev.is_ascii_digit() && current.is_ascii_alphabetic())
            || (prev.is_ascii_uppercase()
                && current.is_ascii_uppercase()
                && next.is_some_and(|c| c.is_ascii_lowercase()));
        if boundary {
            starts.push(chars[i].0);
        }
    }
    starts.push(token.len());
    starts
        .windows(2)
        .filter_map(|w| {
            let part = &token[w[0]..w[1]];
            (!part.is_empty()).then(|| part.to_lowercase())
        })
        .collect()
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for m in TOKEN_RE.find_iter(text) {
        for token in split_identifier(m.as_str()) {
            push_expanded_token(&mut out, token);
        }
    }
    out
}

fn push_expanded_token(out: &mut Vec<String>, token: String) {
    let mut variants = vec![token.clone()];
    variants.extend(light_normalizations(&token));
    for variant in variants {
        if !variant.is_empty() && !out.contains(&variant) {
            out.push(variant);
        }
    }
}

fn light_normalizations(token: &str) -> Vec<String> {
    let mut variants = Vec::new();
    add_morphology(token, &mut variants);
    add_spelling_variants(token, &mut variants);
    add_domain_synonyms(token, &mut variants);
    variants
}

fn add_morphology(token: &str, variants: &mut Vec<String>) {
    if token.len() > 4 && token.ends_with("ies") {
        variants.push(format!("{}y", &token[..token.len() - 3]));
    }
    if token.len() > 4 && token.ends_with("es") {
        variants.push(token[..token.len() - 2].to_owned());
    }
    if token.len() > 3 && token.ends_with('s') {
        variants.push(token[..token.len() - 1].to_owned());
    }
    if token.len() > 5 && token.ends_with("ing") {
        variants.push(token[..token.len() - 3].to_owned());
        variants.push(format!("{}e", &token[..token.len() - 3]));
    }
    if token.len() > 4 && token.ends_with("ed") {
        variants.push(token[..token.len() - 2].to_owned());
        variants.push(format!("{}e", &token[..token.len() - 2]));
    }
}

fn add_spelling_variants(token: &str, variants: &mut Vec<String>) {
    if token.contains("serialis") {
        variants.push(token.replace("serialis", "serializ"));
    }
    if token.contains("serializ") {
        variants.push(token.replace("serializ", "serialis"));
    }
}

fn add_domain_synonyms(token: &str, variants: &mut Vec<String>) {
    match token {
        "auth" => variants.push("authentication".to_owned()),
        "authentication" => variants.push("auth".to_owned()),
        "config" => variants.push("configuration".to_owned()),
        "configuration" => variants.push("config".to_owned()),
        "ctx" => variants.push("context".to_owned()),
        "context" => variants.push("ctx".to_owned()),
        "req" => variants.push("request".to_owned()),
        "request" => variants.push("req".to_owned()),
        "resp" => variants.push("response".to_owned()),
        "response" => variants.push("resp".to_owned()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{light_normalizations, split_identifier, tokenize};

    #[test]
    fn tokenization_matches_identifier_expansion() {
        assert_eq!(
            split_identifier("HandlerStack"),
            vec!["handlerstack", "handler", "stack"]
        );
        assert_eq!(split_identifier("my_func"), vec!["my_func", "my", "func"]);
        assert_eq!(split_identifier("__init__"), vec!["__init__", "init"]);
        assert_eq!(split_identifier("_private"), vec!["_private", "private"]);
        assert_eq!(split_identifier("test_"), vec!["test_", "test"]);
        assert_eq!(split_identifier("_"), vec!["_"]);
        assert_eq!(split_identifier("__"), vec!["__"]);
        assert_eq!(
            tokenize("getHTTPResponse my_func"),
            vec![
                "gethttpresponse",
                "get",
                "http",
                "response",
                "resp",
                "my_func",
                "my",
                "func"
            ]
        );
    }

    #[test]
    fn tokenization_adds_conservative_query_expansions() {
        assert!(light_normalizations("handlers").contains(&"handler".to_owned()));
        assert!(light_normalizations("serialisation").contains(&"serialization".to_owned()));
        assert!(tokenize("auth handlers").contains(&"authentication".to_owned()));
        assert!(tokenize("deserializing").contains(&"deserialize".to_owned()));
    }
}
