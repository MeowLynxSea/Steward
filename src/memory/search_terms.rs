use std::collections::HashSet;

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0x31F0..=0x31FF
            | 0xAC00..=0xD7AF
    )
}

fn flush_ascii(buffer: &mut String, tokens: &mut Vec<String>) {
    if !buffer.is_empty() {
        tokens.push(buffer.to_ascii_lowercase());
        buffer.clear();
    }
}

fn push_cjk_run(run: &mut String, tokens: &mut Vec<String>) {
    if run.is_empty() {
        return;
    }
    tokens.push(run.clone());
    let chars: Vec<char> = run.chars().collect();
    if chars.len() >= 2 {
        for window in chars.windows(2) {
            tokens.push(window.iter().collect());
        }
    }
    run.clear();
}

pub fn tokenize_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii = String::new();
    let mut cjk = String::new();

    for ch in text.chars() {
        if is_cjk(ch) {
            flush_ascii(&mut ascii, &mut tokens);
            cjk.push(ch);
            continue;
        }

        push_cjk_run(&mut cjk, &mut tokens);
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ascii.push(ch);
        } else {
            flush_ascii(&mut ascii, &mut tokens);
        }
    }

    flush_ascii(&mut ascii, &mut tokens);
    push_cjk_run(&mut cjk, &mut tokens);

    dedupe(tokens)
}

pub fn build_search_terms(parts: &[&str]) -> String {
    let mut tokens = Vec::new();
    for part in parts {
        tokens.extend(tokenize_text(part));
    }
    dedupe(tokens).join(" ")
}

pub fn expand_query_terms(query: &str) -> Vec<String> {
    let mut tokens = tokenize_text(query);
    for hint in semantic_hints(query) {
        tokens.extend(tokenize_text(hint));
    }
    dedupe(tokens)
}

pub fn sqlite_match_query(query: &str) -> String {
    let tokens = expand_query_terms(query);
    if tokens.is_empty() {
        "\"memory\"".to_string()
    } else {
        format!(
            "({})",
            tokens
                .into_iter()
                .map(|token| format!("\"{}\"", token.replace('\"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ")
        )
    }
}

pub fn matched_keywords(query: &str, keywords: &[String]) -> Vec<String> {
    let query_terms = expand_query_terms(query)
        .into_iter()
        .collect::<HashSet<_>>();
    let mut matches = Vec::new();
    for keyword in keywords {
        let keyword_terms = tokenize_text(keyword);
        if keyword_terms.iter().any(|term| query_terms.contains(term)) {
            matches.push(keyword.clone());
        }
    }
    dedupe(matches)
}

fn dedupe(tokens: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for token in tokens {
        if token.is_empty() || !seen.insert(token.clone()) {
            continue;
        }
        ordered.push(token);
    }
    ordered
}

fn semantic_hints(query: &str) -> Vec<&'static str> {
    let lowered = query.to_ascii_lowercase();
    let mut hints = Vec::new();

    if query.contains("我是谁")
        || query.contains("我叫什么")
        || query.contains("怎么称呼我")
        || query.contains("记得我叫什么")
        || lowered.contains("who am i")
        || lowered.contains("what is my name")
        || lowered.contains("what's my name")
        || lowered.contains("remember my name")
    {
        hints.push("user identity name profile 名字 称呼 用户");
    }

    if query.contains("你是谁")
        || query.contains("你叫什么")
        || lowered.contains("who are you")
        || lowered.contains("what is your name")
        || lowered.contains("what's your name")
    {
        hints.push("assistant identity name self introduction 助手 名字 自我介绍");
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_ascii_and_cjk() {
        let tokens = tokenize_text("user/name 梦凌汐 hello-world");
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"name".to_string()));
        assert!(tokens.contains(&"梦凌汐".to_string()));
        assert!(tokens.contains(&"梦凌".to_string()));
        assert!(tokens.contains(&"凌汐".to_string()));
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
    }

    #[test]
    fn query_expansion_adds_identity_hints() {
        let tokens = expand_query_terms("我是谁");
        assert!(tokens.contains(&"identity".to_string()));
        assert!(tokens.contains(&"name".to_string()));
        assert!(tokens.contains(&"名字".to_string()));
    }
}
