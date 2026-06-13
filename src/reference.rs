use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReferenceEntry {
    String(String),
    Local { path: String, description: Option<String>, hidden: Option<bool> },
    Git { repository: String, branch: Option<String>, description: Option<String>, hidden: Option<bool> },
}

#[derive(Debug, Clone)]
pub enum ReferenceSource {
    Local { path: PathBuf, description: Option<String>, hidden: Option<bool> },
    Git { repository: String, branch: Option<String>, description: Option<String>, hidden: Option<bool> },
}

#[derive(Debug, Clone)]
pub struct ReferenceInfo {
    pub name: String,
    pub source: ReferenceSource,
}

fn valid_alias(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains(' ') && !name.contains(',')
}

fn is_local(value: &str) -> bool {
    value.starts_with('.') || value.starts_with('/') || value.starts_with('~')
}

fn resolve_path(directory: &str, home: &str, value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else if value.starts_with('/') {
        PathBuf::from(value)
    } else {
        PathBuf::from(directory).join(value)
    }
}

pub fn load_references(
    config_references: &HashMap<String, ReferenceEntry>,
    config_dir: &str,
    home: &str,
) -> Vec<ReferenceInfo> {
    let mut references = Vec::new();

    for (name, entry) in config_references {
        if !valid_alias(name) {
            continue;
        }

        let source = match entry {
            ReferenceEntry::String(s) => {
                if is_local(s) {
                    ReferenceSource::Local {
                        path: resolve_path(config_dir, home, s),
                        description: None,
                        hidden: None,
                    }
                } else {
                    ReferenceSource::Git {
                        repository: s.clone(),
                        branch: None,
                        description: None,
                        hidden: None,
                    }
                }
            }
            ReferenceEntry::Local { path, description, hidden } => {
                ReferenceSource::Local {
                    path: resolve_path(config_dir, home, path),
                    description: description.clone(),
                    hidden: *hidden,
                }
            }
            ReferenceEntry::Git { repository, branch, description, hidden } => {
                ReferenceSource::Git {
                    repository: repository.clone(),
                    branch: branch.clone(),
                    description: description.clone(),
                    hidden: *hidden,
                }
            }
        };

        references.push(ReferenceInfo {
            name: name.clone(),
            source,
        });
    }

    references
}

#[allow(dead_code)]
pub fn format_reference_for_display(info: &ReferenceInfo) -> String {
    match &info.source {
        ReferenceSource::Local { path, description, .. } => {
            if let Some(desc) = description {
                format!("{} <{}>", info.name, desc)
            } else {
                format!("{} <{}>", info.name, path.display())
            }
        }
        ReferenceSource::Git { repository, branch, description, .. } => {
            let branch_str = branch.as_deref().unwrap_or("main");
            if let Some(desc) = description {
                format!("{} <{}#{}> {}", info.name, repository, branch_str, desc)
            } else {
                format!("{} <{}#{}>", info.name, repository, branch_str)
            }
        }
    }
}
