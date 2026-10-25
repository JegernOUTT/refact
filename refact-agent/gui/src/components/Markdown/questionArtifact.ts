import { wrapArtifactHtml } from "./renderUtils";

export const MAX_QUESTION_ANSWER_LENGTH = 8192;

export function questionArtifactHtml(code: string, channel: string): string {
  const doc = new DOMParser().parseFromString(
    wrapArtifactHtml(code),
    "text/html",
  );
  doc
    .querySelectorAll('base, meta[http-equiv="refresh" i]')
    .forEach((node) => node.remove());
  const policy = doc.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content =
    "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";
  const script = doc.createElement("script");
  script.textContent = `(function(){
    var channel=${JSON.stringify(channel).replace(
      /</g,
      "\\u003c",
    )}, answer='', enabled=false;
    window.refactQuestion=Object.freeze({
      getAnswer:function(){return answer;},
      setAnswer:function(value){
        if(!enabled||typeof value!=='string'||value.length>${MAX_QUESTION_ANSWER_LENGTH})return;
        answer=value;
        window.parent.postMessage({type:'refact-question-answer',channel:channel,answer:value},'*');
      }
    });
    window.addEventListener('message',function(event){
      if(event.source!==window.parent)return;
      var data=event.data;
      if(!data||data.type!=='refact-question-state'||data.channel!==channel)return;
      enabled=data.enabled===true;
      window.dispatchEvent(new CustomEvent('refact-question-state',{detail:{answer:answer,enabled:enabled}}));
    });
  })();`;
  doc.head.prepend(policy, script);
  return "<!doctype html>" + doc.documentElement.outerHTML;
}

export function readQuestionArtifactAnswer(
  data: unknown,
  channel: string,
): string | null {
  if (!data || typeof data !== "object") return null;
  const message = data as Record<string, unknown>;
  return message.type === "refact-question-answer" &&
    message.channel === channel &&
    typeof message.answer === "string" &&
    message.answer.length <= MAX_QUESTION_ANSWER_LENGTH
    ? message.answer
    : null;
}
