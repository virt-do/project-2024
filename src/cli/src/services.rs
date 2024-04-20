use crate::types::{Language, LogLevel, YamlConfigFile};
use crate::utils::read_file;
use reqwest::Client;
use serde::Serialize;
use std::error::Error;
#[derive(Serialize, Debug)]
pub struct HttpVmmRequest {
    pub language: Language,
    pub env: String,
    pub code: String,
    pub log_level: LogLevel,
}

impl HttpVmmRequest {
    pub fn new(config: YamlConfigFile) -> Self {
        let code: String = read_file(&config.code_path).expect("Error while reading the code file");
        let env = read_file(&config.env_path).expect("Error while reading the environment file");
        let language = config.language;
        let log_level = config.log_level;
        HttpVmmRequest {
            language,
            env,
            code,
            log_level,
        }
    }

    pub async fn post(request: HttpVmmRequest) -> Result<(), Box<dyn Error>> {
        let client = Client::new();
        let res = client
            .post("http://127.0.0.1:3000/run")
            .body(serde_json::to_string(&request)?)
            .send()
            .await?;
        println!("Response Status: {}", res.status());
        let body = res.text().await?;
        println!("Response body: {}", body);
        Ok(())
    }
}
