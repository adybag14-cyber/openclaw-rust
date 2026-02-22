use regex::Regex;

use crate::config::ToolRuntimeSafetyLayerConfig;
use crate::types::DecisionAction;

#[derive(Debug, Clone)]
pub struct SafetyLayerReport {
    pub risk: u8,
    pub min_action: DecisionAction,
    pub tags: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SafetyTextOutcome {
    pub text: String,
    pub report: SafetyLayerReport,
}

#[derive(Debug, Clone)]
pub struct SafetyLayer {
    cfg: ToolRuntimeSafetyLayerConfig,
    review_patterns: Vec<Regex>,
    block_patterns: Vec<Regex>,
}

impl Default for SafetyLayerReport {
    fn default() -> Self {
        Self {
            risk: 0,
            min_action: DecisionAction::Allow,
            tags: Vec::new(),
            reasons: Vec::new(),
        }
    }
}

impl SafetyLayerReport {
    pub fn merge_into(
        self,
        risk: &mut u8,
        min_action: &mut DecisionAction,
        tags: &mut Vec<String>,
        reasons: &mut Vec<String>,
    ) {
        *risk = risk.saturating_add(self.risk);
        *min_action = max_action(*min_action, self.min_action);
        tags.extend(self.tags);
        reasons.extend(self.reasons);
    }
}

impl SafetyLayer {
    pub fn new(cfg: ToolRuntimeSafetyLayerConfig) -> Self {
        Self {
            cfg,
            review_patterns: vec![
                Regex::new(r"(?i)ignore\s+all\s+previous\s+instructions").expect("valid regex"),
                Regex::new(r"(?i)reveal\s+the\s+system\s+prompt").expect("valid regex"),
                Regex::new(r"(?i)disable\s+safety").expect("valid regex"),
                Regex::new(r"(?i)override\s+developer\s+instructions").expect("valid regex"),
            ],
            block_patterns: vec![
                Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").expect("valid regex"),
                Regex::new(r"(?i)\baws_secret_access_key\b\s*[:=]").expect("valid regex"),
            ],
        }
    }

    pub fn inspect_input(&self, text: &str) -> SafetyLayerReport {
        if !self.cfg.enabled {
            return SafetyLayerReport::default();
        }

        let mut report = SafetyLayerReport::default();
        for pattern in &self.review_patterns {
            if pattern.is_match(text) {
                report.risk = report.risk.saturating_add(15);
                report.min_action = max_action(report.min_action, DecisionAction::Review);
                report.tags.push("safety_layer_review".to_owned());
                report
                    .reasons
                    .push("safety layer found prompt-injection style content".to_owned());
            }
        }
        for pattern in &self.block_patterns {
            if pattern.is_match(text) {
                report.risk = report.risk.saturating_add(40);
                report.min_action = max_action(report.min_action, DecisionAction::Block);
                report.tags.push("safety_layer_block".to_owned());
                report
                    .reasons
                    .push("safety layer found high-risk secret material markers".to_owned());
            }
        }
        report
    }

    pub fn sanitize_output(&self, text: &str) -> SafetyTextOutcome {
        if !self.cfg.enabled {
            return SafetyTextOutcome {
                text: text.to_owned(),
                report: SafetyLayerReport::default(),
            };
        }

        let mut report = self.inspect_input(text);
        let mut output = strip_control_characters(text);
        if output != text {
            report.risk = report.risk.saturating_add(5);
            report.tags.push("safety_layer_control_chars".to_owned());
            report
                .reasons
                .push("output contained control characters removed by safety layer".to_owned());
            report.min_action = max_action(report.min_action, DecisionAction::Review);
        }
        if self.cfg.sanitize_output {
            let truncated = truncate_output(&output, self.cfg.max_output_chars);
            if truncated != output {
                report.risk = report.risk.saturating_add(5);
                report.tags.push("safety_layer_truncated".to_owned());
                report
                    .reasons
                    .push("output truncated by safety layer max_output_chars policy".to_owned());
            }
            output = truncated;
        }

        SafetyTextOutcome {
            text: output,
            report,
        }
    }
}

pub fn max_action(a: DecisionAction, b: DecisionAction) -> DecisionAction {
    if action_rank(a) >= action_rank(b) {
        a
    } else {
        b
    }
}

pub fn action_rank(action: DecisionAction) -> u8 {
    match action {
        DecisionAction::Allow => 0,
        DecisionAction::Review => 1,
        DecisionAction::Block => 2,
    }
}

pub fn truncate_output(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut clipped = text.chars().take(max_chars).collect::<String>();
    clipped.push_str("\n... [truncated by safety layer]");
    clipped
}

fn strip_control_characters(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect::<String>()
}
