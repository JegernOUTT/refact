#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpeechIntent {
    Humor,
    Suggestion,
    Insight,
    Win,
    ErrorAlert,
    Greeting,
    Tour,
    Milestone,
    MemoryPulseCommentary,
    QuestAccept,
    QuestComplete,
}

impl SpeechIntent {
    pub fn as_str(self) -> &'static str {
        match self {
            SpeechIntent::Humor => "speech:humor",
            SpeechIntent::Suggestion => "speech:suggestion",
            SpeechIntent::Insight => "speech:insight",
            SpeechIntent::Win => "speech:win",
            SpeechIntent::ErrorAlert => "speech:error_alert",
            SpeechIntent::Greeting => "speech:greeting",
            SpeechIntent::Tour => "speech:tour",
            SpeechIntent::Milestone => "speech:milestone",
            SpeechIntent::MemoryPulseCommentary => "speech:memory_pulse_commentary",
            SpeechIntent::QuestAccept => "speech:quest_accept",
            SpeechIntent::QuestComplete => "speech:quest_complete",
        }
    }

    pub fn mood(self) -> &'static str {
        match self {
            SpeechIntent::ErrorAlert => "concerned",
            SpeechIntent::Win | SpeechIntent::Milestone | SpeechIntent::QuestComplete => "happy",
            SpeechIntent::Humor => "playful",
            SpeechIntent::Tour | SpeechIntent::Greeting | SpeechIntent::QuestAccept => "excited",
            SpeechIntent::Suggestion
            | SpeechIntent::Insight
            | SpeechIntent::MemoryPulseCommentary => "curious",
        }
    }
}
