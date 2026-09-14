use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    UserInput,
    AgentParent,
    BoardPlanner,
    PlanApproval,
    PermissionEscalation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Pending,
    Answered,
    Rejected,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionContract {
    pub id: String,
    pub kind: QuestionKind,
    pub asker: String,
    pub answerer: String,
    pub correlation_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub state: QuestionState,
    pub asked_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
}

impl QuestionContract {
    pub fn pending(
        id: impl Into<String>,
        kind: QuestionKind,
        asker: impl Into<String>,
        answerer: impl Into<String>,
        correlation_id: impl Into<String>,
        text: impl Into<String>,
        options: Option<Vec<String>>,
        asked_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Self, String> {
        let question = Self {
            id: id.into(),
            kind,
            asker: asker.into(),
            answerer: answerer.into(),
            correlation_id: correlation_id.into(),
            text: text.into(),
            options,
            state: QuestionState::Pending,
            asked_at,
            expires_at,
            answer: None,
            answered_at: None,
        };
        question.validate()?;
        Ok(question)
    }

    pub fn validate(&self) -> Result<(), String> {
        if [
            self.id.as_str(),
            self.asker.as_str(),
            self.answerer.as_str(),
            self.correlation_id.as_str(),
            self.text.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err(
                "question identity, principals, correlation and text must be nonempty".into(),
            );
        }
        if self
            .expires_at
            .is_some_and(|expires_at| expires_at <= self.asked_at)
        {
            return Err("question expiry must be after creation".into());
        }
        if self.state == QuestionState::Answered
            && (self.answer.as_deref().is_none_or(str::is_empty) || self.answered_at.is_none())
        {
            return Err("answered questions require answer text and timestamp".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_question_family_roundtrips_through_one_contract() {
        for kind in [
            QuestionKind::UserInput,
            QuestionKind::AgentParent,
            QuestionKind::BoardPlanner,
            QuestionKind::PlanApproval,
            QuestionKind::PermissionEscalation,
        ] {
            let question = QuestionContract::pending(
                "q",
                kind,
                "asker",
                "answerer",
                "correlation",
                "Continue?",
                None,
                Utc::now(),
                None,
            )
            .unwrap();
            let decoded: QuestionContract =
                serde_json::from_value(serde_json::to_value(&question).unwrap()).unwrap();
            assert_eq!(decoded, question);
        }
    }
}
