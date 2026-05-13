//! Prompt builder with `{{var}}` substitution.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;

/// Builds a final prompt string from a template + variable bindings.
/// Variables in the template have the form `{{name}}` (Handlebars-like).
/// Unknown variables are kept as-is in the output (useful for partial fills).
pub struct PromptBuilder {
    template: String,
    vars: BTreeMap<String, String>,
}

impl PromptBuilder {
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
            vars: BTreeMap::new(),
        }
    }

    /// Sets `{{name}}` → `value`. Returns self for chaining.
    pub fn var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }

    /// Renders the template, substituting all known variables.
    /// Unknown variables remain literal in the output (e.g., `{{unknown}}`).
    pub fn render(&self) -> String {
        static VAR_RE: OnceLock<Regex> = OnceLock::new();
        #[allow(
            clippy::unwrap_used,
            reason = "static regex literal; validity guarded by unit tests"
        )]
        let re =
            VAR_RE.get_or_init(|| Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_-]*)\s*\}\}").unwrap());
        re.replace_all(&self.template, |caps: &regex::Captures| -> String {
            let name = &caps[1];
            self.vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_builder_substitutes_known_vars() {
        let out = PromptBuilder::new("Hello {{name}}, see {{topic}}")
            .var("name", "Ada")
            .var("topic", "wiki")
            .render();
        assert_eq!(out, "Hello Ada, see wiki");
    }

    #[test]
    fn prompt_builder_keeps_unknown_vars_literal() {
        let out = PromptBuilder::new("{{name}}: {{missing}}")
            .var("name", "A")
            .render();
        assert_eq!(out, "A: {{missing}}");
    }

    #[test]
    fn prompt_builder_handles_whitespace() {
        let out = PromptBuilder::new("{{ name }}").var("name", "X").render();
        assert_eq!(out, "X");
    }

    #[test]
    fn prompt_builder_supports_underscores_and_dashes() {
        let out = PromptBuilder::new("{{a_b-c}}").var("a_b-c", "v").render();
        assert_eq!(out, "v");
    }

    #[test]
    fn prompt_builder_no_vars_no_change() {
        let template = "no variables here, just text.";
        let out = PromptBuilder::new(template).render();
        assert_eq!(out, template);
    }
}
