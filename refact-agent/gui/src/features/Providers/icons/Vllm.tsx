import { FC, SVGProps } from "react";

export const VllmIcon: FC<SVGProps<SVGSVGElement>> = (props) => {
  return (
    <svg
      width="30"
      height="30"
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      <rect x="3" y="3" width="18" height="18" rx="4" fill="currentColor" opacity="0.12" />
      <path
        d="M7.5 9.2L10.5 15H13.5L16.5 9.2"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M7.5 15H16.5"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
};
