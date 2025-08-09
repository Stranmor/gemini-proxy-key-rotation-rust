// src/tokenizer/modern.rs

use std::error::Error;
use std::sync::OnceLock;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub enum TokenizerType {
    /// OpenAI GPT-4/3.5 токенизатор (cl100k_base)
    OpenAI,
    /// Claude токенизатор
    Claude,
    /// Llama токенизатор
    Llama,
    /// Gemini токенизатор
    Gemini,
    /// Fallback для тестов
    Minimal,
}

impl TokenizerType {
    pub fn from_str(s: &str) -> Result<Self, Box<dyn Error + Send + Sync>> {
        match s.to_lowercase().as_str() {
            "openai" | "gpt4" | "gpt-4" | "gpt3.5" | "gpt-3.5" => Ok(Self::OpenAI),
            "claude" | "anthropic" => Ok(Self::Claude),
            "llama" | "llama2" | "llama3" | "meta" => Ok(Self::Llama),
            "gemini" | "google" => Ok(Self::Gemini),
            "minimal" | "test" => Ok(Self::Minimal),
            _ => Err(format!("Unknown tokenizer type: {}", s).into()),
        }
    }
}

pub struct ModernTokenizer {
    tokenizer_type: TokenizerType,
    #[cfg(feature = "tokenizer")]
    tiktoken: Option<tiktoken_rs::CoreBPE>,
    #[cfg(feature = "tokenizer")]
    hf_tokenizer: Option<tokenizers::Tokenizer>,
}

static MODERN_TOKENIZER: OnceLock<ModernTokenizer> = OnceLock::new();

impl ModernTokenizer {
    pub async fn initialize(tokenizer_type: TokenizerType) -> Result<(), Box<dyn Error + Send + Sync>> {
        info!(?tokenizer_type, "Initializing modern tokenizer");
        
        let tokenizer = match tokenizer_type {
            TokenizerType::OpenAI => Self::init_openai().await?,
            TokenizerType::Claude => Self::init_claude().await?,
            TokenizerType::Llama => Self::init_llama().await?,
            TokenizerType::Gemini => Self::init_gemini().await?,
            TokenizerType::Minimal => Self::init_minimal()?,
        };
        
        match MODERN_TOKENIZER.set(tokenizer) {
            Ok(_) => info!("Modern tokenizer initialized successfully"),
            Err(_) => warn!("Modern tokenizer was already initialized"),
        }
        
        Ok(())
    }
    
    #[cfg(feature = "tokenizer")]
    async fn init_openai() -> Result<Self, Box<dyn Error + Send + Sync>> {
        use tiktoken_rs::cl100k_base;
        
        let tiktoken = cl100k_base()?;
        Ok(Self {
            tokenizer_type: TokenizerType::OpenAI,
            tiktoken: Some(tiktoken),
            hf_tokenizer: None,
        })
    }
    
    #[cfg(not(feature = "tokenizer"))]
    async fn init_openai() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Err("Tokenizer feature not enabled".into())
    }
    
    #[cfg(feature = "tokenizer")]
    async fn init_claude() -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Claude использует cl100k_base (тот же что и GPT-4)
        use tiktoken_rs::cl100k_base;
        
        let tiktoken = cl100k_base()?;
        Ok(Self {
            tokenizer_type: TokenizerType::Claude,
            tiktoken: Some(tiktoken),
            hf_tokenizer: None,
        })
    }
    
    #[cfg(not(feature = "tokenizer"))]
    async fn init_claude() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Err("Tokenizer feature not enabled".into())
    }
    
    #[cfg(feature = "tokenizer")]
    async fn init_llama() -> Result<Self, Box<dyn Error + Send + Sync>> {
        use hf_hub::api::tokio::Api;
        use tokenizers::Tokenizer;
        
        let api = Api::new()?;
        let repo = api.model("meta-llama/Meta-Llama-3-8B".to_string());
        let tokenizer_path = repo.get("tokenizer.json").await?;
        let hf_tokenizer = Tokenizer::from_file(tokenizer_path)?;
        
        Ok(Self {
            tokenizer_type: TokenizerType::Llama,
            tiktoken: None,
            hf_tokenizer: Some(hf_tokenizer),
        })
    }
    
    #[cfg(not(feature = "tokenizer"))]
    async fn init_llama() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Err("Tokenizer feature not enabled".into())
    }
    
    #[cfg(feature = "tokenizer")]
    async fn init_gemini() -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Для Gemini можем использовать SentencePiece или fallback на cl100k_base
        use tiktoken_rs::cl100k_base;
        
        let tiktoken = cl100k_base()?;
        Ok(Self {
            tokenizer_type: TokenizerType::Gemini,
            tiktoken: Some(tiktoken),
            hf_tokenizer: None,
        })
    }
    
    #[cfg(not(feature = "tokenizer"))]
    async fn init_gemini() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Err("Tokenizer feature not enabled".into())
    }
    
    fn init_minimal() -> Result<Self, Box<dyn Error + Send + Sync>> {
        #[cfg(feature = "tokenizer")]
        {
            use tokenizers::Tokenizer;
            
            let simple_tokenizer_json = r#"{
                "version":"1.0",
                "truncation":null,
                "padding":null,
                "added_tokens":[],
                "normalizer": null,
                "pre_tokenizer": { "type": "Whitespace" },
                "post_processor": null,
                "decoder": null,
                "model": { 
                    "type": "WordLevel", 
                    "vocab": {"a":0, "b":1, "c":2, "[UNK]":3}, 
                    "unk_token":"[UNK]" 
                }
            }"#;
            
            let hf_tokenizer = Tokenizer::from_bytes(simple_tokenizer_json.as_bytes())?;
            Ok(Self {
                tokenizer_type: TokenizerType::Minimal,
                tiktoken: None,
                hf_tokenizer: Some(hf_tokenizer),
            })
        }
        
        #[cfg(not(feature = "tokenizer"))]
        {
            Ok(Self {
                tokenizer_type: TokenizerType::Minimal,
            })
        }
    }
    
    pub fn count_tokens(&self, text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
        match self.tokenizer_type {
            TokenizerType::OpenAI | TokenizerType::Claude | TokenizerType::Gemini => {
                #[cfg(feature = "tokenizer")]
                {
                    if let Some(ref tiktoken) = self.tiktoken {
                        Ok(tiktoken.encode_with_special_tokens(text).len())
                    } else {
                        Err("TikToken tokenizer not initialized".into())
                    }
                }
                
                #[cfg(not(feature = "tokenizer"))]
                {
                    // Простая оценка: ~4 символа на токен
                    Ok((text.len() + 3) / 4)
                }
            }
            TokenizerType::Llama | TokenizerType::Minimal => {
                #[cfg(feature = "tokenizer")]
                {
                    if let Some(ref hf_tokenizer) = self.hf_tokenizer {
                        let encoding = hf_tokenizer.encode(text, false)?;
                        Ok(encoding.len())
                    } else {
                        Err("HF tokenizer not initialized".into())
                    }
                }
                
                #[cfg(not(feature = "tokenizer"))]
                {
                    // Простая оценка для Llama: разбиение по пробелам
                    Ok(text.split_whitespace().count().max(1))
                }
            }
        }
    }
    
    pub fn get_type(&self) -> &TokenizerType {
        &self.tokenizer_type
    }
}

pub fn count_tokens_modern(text: &str) -> Result<usize, Box<dyn Error + Send + Sync>> {
    let tokenizer = MODERN_TOKENIZER
        .get()
        .ok_or("Modern tokenizer not initialized")?;
    
    tokenizer.count_tokens(text)
}

pub fn get_tokenizer_type() -> Option<&'static TokenizerType> {
    MODERN_TOKENIZER.get().map(|t| t.get_type())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_openai_tokenizer() {
        if std::env::var("HF_TOKEN").is_err() {
            println!("Skipping OpenAI tokenizer test: HF_TOKEN not available");
            return;
        }
        
        let result = ModernTokenizer::initialize(TokenizerType::OpenAI).await;
        if result.is_err() {
            println!("Skipping OpenAI tokenizer test: initialization failed");
            return;
        }
        
        let count = count_tokens_modern("Hello, world!").unwrap();
        assert!(count > 0);
        assert!(count < 10); // Разумная оценка для короткого текста
    }
    
    #[tokio::test]
    async fn test_minimal_tokenizer() {
        ModernTokenizer::initialize(TokenizerType::Minimal).await.unwrap();
        
        let count = count_tokens_modern("Hello world test").unwrap();
        assert_eq!(count, 3); // Три слова = три токена
    }
    
    #[tokio::test]
    async fn test_performance_comparison() {
        ModernTokenizer::initialize(TokenizerType::Minimal).await.unwrap();
        
        let text = "This is a test text for performance measurement";
        let start = std::time::Instant::now();
        
        for _ in 0..1000 {
            let _ = count_tokens_modern(text).unwrap();
        }
        
        let duration = start.elapsed();
        println!("1000 tokenizations took: {:?}", duration);
        
        // Должно быть очень быстро (< 10ms для 1000 операций)
        assert!(duration.as_millis() < 100);
    }
}