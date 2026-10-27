declare global {
  interface Window {
    postIntellijMessage?: (message: Record<string, unknown>) => void;
    acquireVsCodeApi?: () => {
      postMessage: (message: Record<string, unknown>) => void;
    };
  }
}

let postMessage: Window["postMessage"] | undefined;
let acquireVsCodeApi: Window["acquireVsCodeApi"];
const webPostMessage = (message: Record<string, unknown>) =>
  window.postMessage(message, "*");

function setUpPostMessage() {
  if (window.acquireVsCodeApi) {
    if (acquireVsCodeApi !== window.acquireVsCodeApi || !postMessage) {
      acquireVsCodeApi = window.acquireVsCodeApi;
      postMessage = window.acquireVsCodeApi().postMessage;
    }
    return postMessage;
  } else if (window.postIntellijMessage) {
    return window.postIntellijMessage;
  }
  return webPostMessage;
}

export const usePostMessage = () => {
  return setUpPostMessage();
};
