use std::time::Duration;

use serde_json::Value;

pub const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub const OLLAMA_DOWNLOAD_URL: &str = "https://ollama.com/download";

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OllamaInfo {
    pub reachable: bool,
    pub models: Vec<String>,
    pub error: Option<String>,
}

pub fn probe(timeout: Duration) -> OllamaInfo {
    let client = match reqwest::blocking::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return OllamaInfo {
                error: Some(error.to_string()),
                ..OllamaInfo::default()
            }
        }
    };

    match client
        .get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
    {
        Ok(body) => match parse_tags(&body) {
            Some(models) => OllamaInfo {
                reachable: true,
                models,
                error: None,
            },
            None => OllamaInfo {
                error: Some("Ollama returned an unexpected response".into()),
                ..OllamaInfo::default()
            },
        },
        Err(error) => OllamaInfo {
            error: Some(error.to_string()),
            ..OllamaInfo::default()
        },
    }
}

pub fn parse_tags(body: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let models = value.get("models")?.as_array()?;
    Some(
        models
            .iter()
            .filter_map(|model| model.get("name")?.as_str().map(str::to_owned))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_tags_endpoint() {
        let body = r#"{"models":[{"name":"qwen2.5:1.5b-instruct"},{"name":"llama3:8b"}]}"#;
        assert_eq!(
            parse_tags(body).unwrap(),
            vec!["qwen2.5:1.5b-instruct".to_string(), "llama3:8b".to_string()]
        );
    }

    #[test]
    fn rejects_unexpected_bodies() {
        assert!(parse_tags("nope").is_none());
        assert!(parse_tags(r#"{"models":"nope"}"#).is_none());
        assert!(parse_tags(r#"{"other":[]}"#).is_none());
    }
}
