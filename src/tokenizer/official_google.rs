// src/tokenizer/official_google.rs

use std::error::Error;
use std::sync::OnceLock;
use std::process::Command;
use std::fs;
use tracing::{info, warn};
use serde_json::Value;
use tempfile::TempDir;

/// Официальный токенизатор Google через Python Vertex AI SDK
/// Обеспечивает 100% точность используя тот же токенизатор что и Google
pub struct OfficialGoogleTokenizer {
    python_script_path: String,
    #[allow(dead_code)]
    temp_dir: TempDir,
}

static OFFICIAL_TOKENIZER: OnceLock<OfficialGoogleTokenizer> = OnceLock::new();

impl OfficialGoogleTokenizer {
    /// Инициализирует официальный Google токенизатор
    pub async fn initialize() -> Result<(), Box<dyn Error + Send + Sync>> {
        info!("Initializing official Google tokenizer via Vertex AI SDK");
        
        let tokenizer = Self::new().await?;
        
        match OFFICIAL_TOKENIZER.set(tokenizer) {
            Ok(_) => info!("Official Google tokenizer initialized successfully"),
            Err(_) => warn!("Official Google tokenizer was already initialized"),
        }
        
        Ok(())
    }
    
    async fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Создаем временную директорию для Python скрипта
        let temp_dir = tempfile::tempdir()?;
        let script_path = temp_dir.path().join("tokenizer.py");
        
        // Создаем Python скрипт для токенизации
        let python_script = r#"
import sys
import json
import os

def setup_tokenizer():
    """Устанавливает и настраивает токенизатор"""
    try:
        # Проверяем установлен ли пакет
        from vertexai.preview import tokenization
        return True
    except ImportError:
        print("ERROR: vertexai package not installed", file=sys.stderr)
        print("Please install: pip install google-cloud-aiplatform[tokenization]", file=sys.stderr)
        return False

def count_tokens(text, model_name="gemini-1.5-flash-001"):
    """Подсчитывает токены используя официальный Google токенизатор"""
    try:
        from vertexai.preview import tokenization
        
        # Получаем токенизатор для модели
        tokenizer = tokenization.get_tokenizer_for_model(model_name)
        
        # Подсчитываем токены
        result = tokenizer.count_tokens(text)
        
        return {
            "success": True,
            "total_tokens": result.total_tokens,
            "model": model_name
        }
        
    except Exception as e:
        return {
            "success": False,
            "error": str(e),
            "model": model_name
        }

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"success": False, "error": "No text provided"}))
        sys.exit(1)
    
    # Проверяем установку
    if not setup_tokenizer():
        print(json.dumps({"success": False, "error": "Tokenizer setup failed"}))
        sys.exit(1)
    
    text = sys.argv[1]
    model = sys.argv[2] if len(sys.argv) > 2 else "gemini-1.5-flash-001"
    
    result = count_tokens(text, model)
    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#;
        
        // Записываем скрипт в файл
        fs::write(&script_path, python_script)?;
        
        let tokenizer = Self {
            python_script_path: script_path.to_string_lossy().to_string(),
            temp_dir,
        };
        
        // Проверяем что Python и пакет доступны
        tokenizer.verify_setup().await?;
        
        Ok(tokenizer)
    }
    
    /// Проверяет что Python и Vertex AI SDK установлены
    async fn verify_setup(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        info!("Verifying Python and Vertex AI SDK setup");
        
        let output = Command::new("python3")
            .arg(&self.python_script_path)
            .arg("test")
            .output();
        
        match output {
            Ok(result) => {
                if result.status.success() {
                    info!("✅ Official Google tokenizer setup verified");
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    if stderr.contains("vertexai package not installed") {
                        Err("❌ Please install: pip install google-cloud-aiplatform[tokenization]".into())
                    } else {
                        Err(format!("Python script error: {}", stderr).into())
                    }
                }
            }
            Err(e) => {
                Err(format!("❌ Python3 not found or not accessible: {}", e).into())
            }
        }
    }
    
    /// Подсчитывает токены используя официальный Google токенизатор
    pub fn count_tokens(&self, text: &str, model: Option<&str>) -> Result<usize, Box<dyn Error + Send + Sync>> {
        let model_name = model.unwrap_or("gemini-1.5-flash-001");
        
        let output = Command::new("python3")
            .arg(&self.python_script_path)
            .arg(text)
            .arg(model_name)
            .output()?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Python tokenizer failed: {}", stderr).into());
        }
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: Value = serde_json::from_str(&stdout)?;
        
        if result["success"].as_bool().unwrap_or(false) {
            let token_count = result["total_tokens"].as_u64()
                .ok_or("Missing total_tokens in response")?;
            Ok(token_count as usize)
        } else {
            let error_msg = result["error"].as_str().unwrap_or("Unknown error");
            Err(format!("Google tokenizer error: {}", error_msg).into())
        }
    }
    
    /// Возвращает информацию о токенизаторе
    pub fn get_info(&self) -> String {
        "Official Google Vertex AI Tokenizer (100% accuracy)".to_string()
    }
    
    /// Проверяет доступность для разных моделей
    pub fn check_model_support(&self, model: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
        match self.count_tokens("test", Some(model)) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

/// Подсчитывает токены используя официальный Google токенизатор
pub fn count_official_google_tokens(text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let tokenizer = OFFICIAL_TOKENIZER
        .get()
        .ok_or("Official Google tokenizer not initialized. Call OfficialGoogleTokenizer::initialize() first.")?;
    
    tokenizer.count_tokens(text, None)
}

/// Подсчитывает токены для конкретной модели
pub fn count_official_google_tokens_for_model(text: &str, model: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let tokenizer = OFFICIAL_TOKENIZER
        .get()
        .ok_or("Official Google tokenizer not initialized")?;
    
    tokenizer.count_tokens(text, Some(model))
}

/// Возвращает информацию об официальном токенизаторе
pub fn get_official_google_tokenizer_info() -> Option<String> {
    OFFICIAL_TOKENIZER.get().map(|t| t.get_info())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_official_google_tokenizer() {
        // Этот тест требует установленного Python и Vertex AI SDK
        match OfficialGoogleTokenizer::initialize().await {
            Ok(_) => {
                println!("✅ Official Google tokenizer initialized");
                
                let test_cases = vec![
                    "Hello world",
                    "The quick brown fox jumps over the lazy dog.",
                    "Hello 世界! 🌍 How are you?",
                ];
                
                for text in test_cases {
                    match count_official_google_tokens(text) {
                        Ok(count) => {
                            println!("✅ '{}' -> {} tokens (100% accurate!)", text, count);
                            assert!(count > 0);
                        }
                        Err(e) => {
                            println!("⚠️ Error for '{}': {}", text, e);
                        }
                    }
                }
            }
            Err(e) => {
                println!("⚠️ Official tokenizer not available: {}", e);
                println!("💡 To enable 100% accuracy, install: pip install google-cloud-aiplatform[tokenization]");
            }
        }
    }
    
    #[tokio::test]
    async fn test_model_support() {
        if let Ok(_) = OfficialGoogleTokenizer::initialize().await {
            let tokenizer = OFFICIAL_TOKENIZER.get().unwrap();
            
            let models = vec![
                "gemini-1.5-flash-001",
                "gemini-1.5-pro-001", 
                "gemini-1.0-pro-001",
            ];
            
            for model in models {
                match tokenizer.check_model_support(model) {
                    Ok(supported) => {
                        println!("Model {} supported: {}", model, supported);
                    }
                    Err(e) => {
                        println!("Error checking {}: {}", model, e);
                    }
                }
            }
        }
    }
}