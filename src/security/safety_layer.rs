use crate::types::DecisionAction;

#[derive(Debug, Clone)]
pub struct SafetyLayerReport {
    pub risk: u8,
    pub min_action: DecisionAction,
    pub tags: Vec<String>,
    pub reasons: Vec<String>,
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
