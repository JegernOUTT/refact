import { FC, SVGProps } from "react";

/** A compact train/gateway mark inspired by LiteLLM's public train identity. */
export const LiteLLMIcon: FC<SVGProps<SVGSVGElement>> = (props) => (
  <svg
    width="30"
    height="30"
    viewBox="0 0 30 30"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    xmlns="http://www.w3.org/2000/svg"
    {...props}
  >
    <path d="M7 20V9.5C7 7.6 8.6 6 10.5 6h9C21.4 6 23 7.6 23 9.5V20" />
    <path d="M7 13h16M10 9.5h10M9 20h12l-2 4H11l-2-4Z" />
    <circle cx="11" cy="17" r="1" fill="currentColor" stroke="none" />
    <circle cx="19" cy="17" r="1" fill="currentColor" stroke="none" />
    <path d="M4 24h4m14 0h4" />
  </svg>
);
