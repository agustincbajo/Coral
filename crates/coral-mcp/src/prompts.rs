//! MCP `Prompt` catalog — templated prompts the agent can request.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptDescriptor {
    pub name: String,
    pub description: String,
    /// Template body. v0.19 wave 2 substitutes parameters from the
    /// MCP `prompts/get` request.
    pub template: String,
    pub arguments: Vec<PromptArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

pub struct PromptCatalog;

impl PromptCatalog {
    pub fn list() -> Vec<PromptDescriptor> {
        vec![
            PromptDescriptor {
                name: "onboard".into(),
                description: "Reading path tailored to a developer profile (backend, frontend, on-call, …).".into(),
                template: "You are a new {{profile}} developer joining this project. Read these wiki pages in order: {{slugs}}. After each page, summarize what you learned in 2 bullet points.".into(),
                arguments: vec![PromptArgument {
                    name: "profile".into(),
                    description: "backend | frontend | sre | on-call".into(),
                    required: true,
                }],
            },
            PromptDescriptor {
                name: "cross_repo_trace".into(),
                description: "Explain a flow that crosses multiple repos in this multi-repo project.".into(),
                template: "Trace the flow `{{flow}}` across the repos in this project. For each step, cite the wiki slug that documents it. Highlight any cross-repo boundary crossings.".into(),
                arguments: vec![PromptArgument {
                    name: "flow".into(),
                    description: "Name of the flow (e.g. `user_signup`, `payment_capture`)".into(),
                    required: true,
                }],
            },
            PromptDescriptor {
                name: "code_review".into(),
                description: "Review a pull request against the wiki to flag drift from documented invariants.".into(),
                template: "Review PR #{{pr_number}} in repo `{{repo}}` against the wiki. Flag any code change that contradicts the documented invariants (page slugs cited).".into(),
                arguments: vec![
                    PromptArgument {
                        name: "repo".into(),
                        description: "Repo name from coral.toml [[repos]]".into(),
                        required: true,
                    },
                    PromptArgument {
                        name: "pr_number".into(),
                        description: "GitHub pull request number".into(),
                        required: true,
                    },
                ],
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_three_prompts() {
        let prompts = PromptCatalog::list();
        assert_eq!(prompts.len(), 3);
        let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"onboard"));
        assert!(names.contains(&"cross_repo_trace"));
        assert!(names.contains(&"code_review"));
    }

    #[test]
    fn every_prompt_declares_its_template_and_args() {
        for p in PromptCatalog::list() {
            assert!(!p.template.is_empty());
            assert!(
                !p.arguments.is_empty(),
                "prompt {} has no arguments",
                p.name
            );
        }
    }
}
