import { createContext } from "react";

export type QuestionArtifactBinding = {
  instance: string;
  enabled: boolean;
  onAnswer: (answer: string) => void;
};

export const QuestionArtifactContext =
  createContext<QuestionArtifactBinding | null>(null);
