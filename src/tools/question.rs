use super::{Tool, ToolContext, ToolResult};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct QuestionTool;

impl QuestionTool {
    pub async fn ask_user(questions: &[Question]) -> Result<Vec<Answer>> {
        let mut answers = Vec::new();
        for q in questions {
            let options_str = q
                .options
                .as_ref()
                .map(|opts| {
                    opts.iter()
                        .map(|o| format!("  {} - {}", o.label, o.description.as_deref().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();

            eprintln!("\n=== Question from LLM ===");
            eprintln!("{}", q.question);
            if let Some(header) = &q.header {
                eprintln!("({})", header);
            }
            if !options_str.is_empty() {
                eprintln!("Options:\n{}", options_str);
            }
            eprintln!("Enter your answer (or press Enter for default):");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let answer = input.trim().to_string();

            let selected = if answer.is_empty() {
                q.options
                    .as_ref()
                    .and_then(|opts| opts.first().map(|o| o.label.clone()))
                    .unwrap_or_default()
            } else {
                answer
            };
            answers.push(Answer {
                question: q.question.clone(),
                answer: selected,
            });
        }
        Ok(answers)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Question {
    pub question: String,
    pub header: Option<String>,
    pub options: Option<Vec<OptionItem>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OptionItem {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub question: String,
    pub answer: String,
}

#[async_trait]
impl Tool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Ask the user questions to gather information, preferences, or decisions. Presents interactive questions in the terminal."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "Complete question to ask the user"
                            },
                            "header": {
                                "type": ["string", "null"],
                                "description": "Very short label for the question (max 30 chars)"
                            },
                            "options": {
                                "type": ["array", "null"],
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "Display text (1-5 words, concise)"
                                        },
                                        "description": {
                                            "type": ["string", "null"],
                                            "description": "Explanation of choice"
                                        }
                                    },
                                    "required": ["label"]
                                },
                                "description": "Available choices"
                            }
                        },
                        "required": ["question"]
                    },
                    "description": "Questions to ask the user"
                }
            },
            "required": ["questions"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let questions: Vec<Question> = serde_json::from_value(args["questions"].clone())
            .context("Invalid 'questions' format")?;

        let answers = QuestionTool::ask_user(&questions).await?;

        let mut output = String::new();
        for a in &answers {
            output.push_str(&format!("\"{}\"=\"{}\"\n", a.question, a.answer));
        }

        Ok(ToolResult {
            title: "Questions answered".to_string(),
            output,
            metadata: json!({"answer_count": answers.len()}),
        })
    }
}
