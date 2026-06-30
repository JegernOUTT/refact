import React from "react";
import { JobBuilder, type JobBuilderFormData } from "./JobBuilder";

export type CronCreateFormData = JobBuilderFormData;

export type CronCreateFormProps = React.ComponentProps<typeof JobBuilder>;

export const CronCreateForm: React.FC<CronCreateFormProps> = (props) => (
  <JobBuilder {...props} />
);
