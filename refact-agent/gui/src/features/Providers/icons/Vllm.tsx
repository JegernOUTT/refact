import { FC, SVGProps } from "react";

// Official vLLM logo from github.com/vllm-project/media-kit (vLLM-Logo.svg)
// Scaled from 96x96 to 24x24, simplified, monochrome for theme adaptability
export const VllmIcon: FC<SVGProps<SVGSVGElement>> = (props) => {
  return (
    <svg
      width="30"
      height="30"
      viewBox="0 0 24 24"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
      {...props}
    >
      <path d="M10.262 6.823L10.262 20.801L2.609 6.823Z" />
      <path d="M10.556 7.118L10.556 21.096L4.787 7.118Z" opacity="0.5" />
      <path d="M10.262 20.65L15.422 20.65L18.163 5.014L13.611 10.815Z" />
      <path d="M10.556 20.944L15.422 20.944L18.163 5.303L13.611 11.105Z" opacity="0.5" />
    </svg>
  );
};
