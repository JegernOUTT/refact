import { FC, SVGProps } from "react";

export const OpenCodeIcon: FC<SVGProps<SVGSVGElement>> = (props) => {
  return (
    <svg
      width="30px"
      height="30px"
      viewBox="0 0 30 30"
      xmlns="http://www.w3.org/2000/svg"
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      {...props}
    >
      <path d="M8.5 8.5 3 15l5.5 6.5" />
      <path d="M21.5 8.5 27 15l-5.5 6.5" />
      <path d="M12.5 23 17.5 7" />
      <path d="M10 15h10" />
    </svg>
  );
};
