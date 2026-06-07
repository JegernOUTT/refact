import{j as r}from"./jsx-runtime-CKrituN3.js";import{s as F,P as K,T as W,a as Q}from"./Theme-pgwu0iAo.js";import{A as q}from"./AbortControllers-DJZE2PT_.js";import{g as e,a as n,b as t,c as s,d as o,e as M,n as a,C as z}from"./msw-BPPRbmLt.js";import{C as _}from"./index-WNe4qxV_.js";import{e as J}from"./flex-DhkSe5Wr.js";import"./index-CBqU2yxZ.js";import"./_commonjsHelpers-BosuxZz1.js";/* empty css                   */import"./v4-CQkTLCs1.js";import"./react-icons.esm-Dxg5JPe5.js";import"./get-margin-styles-oNnhyulp.js";import"./extends-CF3RwP-h.js";import"./index-BtM5VmRH.js";import"./select.module-DZDgHAIL.js";import"./index-CZwtOYEC.js";import"./index-DW5OYE7F.js";import"./theme-Bj_5e_Fh.js";import"./index-BAnWuuZQ.js";import"./index-zLxn_asd.js";import"./extract-props-H8xDRupP.js";import"./require-react-element-BKSviGtP.js";import"./index-BWbEzEeM.js";import"./index-Da_Rp7sG.js";import"./index-DiU2Igp9.js";import"./icons-CpZLwehs.js";import"./tooltip-DnsKDLSp.js";import"./button-CG-dCU1M.js";import"./iframe-ULWOz-Wb.js";import"../sb-preview/runtime.js";import"./text-field-DFysKD6w.js";import"./heading-DzHf5iNZ.js";import"./scroll-area-d_jmEDiA.js";import"./get-subtree-uFnELUcI.js";import"./box-ZI7Prd5t.js";import"./ScrollArea-BvZIEMR-.js";import"./container-DUjqJaxJ.js";import"./Checkbox-BeXsbNQS.js";import"./index-DslcL7yX.js";import"./AnimatedText-BOmUW7d4.js";import"./index-BgSubfv6.js";import"./index-D3ylJrlI.js";import"./decorators-Bnaor6Ku.js";const V=({thread:R,config:G})=>{const h=R??{id:"test",model:"gpt-4o",messages:[],new_chat_suggested:{wasSuggested:!1}},g=h.id,N=F({chat:{current_thread_id:g,open_thread_ids:[g],threads:{[g]:{thread:h,streaming:!1,waiting_for_response:!1,prevent_send:!1,error:null,queued_items:[],send_immediately:!1,attached_images:[],attached_text_files:[],background_agents:{},confirmation:{pause:!1,pause_reasons:[],status:{wasInteracted:!1,confirmationStatus:!0}},snapshot_received:!0,task_widget_expanded:!1,memory_enrichment_user_touched:!1,manual_preview_items:[],manual_preview_ran:!1}},max_new_tokens:4096,tool_use:"agent",system_prompt:{},sse_refresh_requested:null,stream_version:0},config:G});return r.jsx(K,{store:N,children:r.jsx(W,{children:r.jsx(q,{children:r.jsx(J,{direction:"column",align:"stretch",height:"100dvh",children:r.jsx(Q,{unCalledTools:!1,host:"web",tabbed:!1,backFromChat:()=>({}),maybeSendToSidebar:()=>({})})})})})})},Ge={title:"Chat",component:V,parameters:{msw:{handlers:[e,n,t,s,o,M]}},argTypes:{}},l={},i={args:{thread:_.threads[_.current_thread_id].thread}},c={args:{config:{host:"ide",lspPort:8001,themeProps:{},features:{vecdb:!0}}},parameters:{msw:{handlers:[e,n,t,s,o,a]}}},m={args:{thread:z,config:{host:"ide",lspPort:8001,themeProps:{},features:{vecdb:!0}}},parameters:{msw:{handlers:[e,n,t,s,o,a]}}},d={args:{thread:{id:"test",model:"gpt-4o",messages:[{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"}],new_chat_suggested:{wasSuggested:!1}}},parameters:{msw:{handlers:[e,n,t,s,o,a]}}},u={args:{thread:{id:"test",model:"gpt-4o",messages:[{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"},{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"},{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"},{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"}],new_chat_suggested:{wasSuggested:!1}}},parameters:{msw:{handlers:[e,n,t,s,o,a]}}},p={args:{thread:{id:"test",model:"gpt-4o",messages:[{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"},{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"},{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋"},{role:"assistant",content:"👋"},{role:"user",content:"Hello"},{role:"assistant",content:"Hi"},{role:"user",content:"👋",compression_strength:"low"},{role:"assistant",content:"👋"}],new_chat_suggested:{wasSuggested:!1}}},parameters:{msw:{handlers:[e,n,t,s,o,a]}}};var D,C,f;l.parameters={...l.parameters,docs:{...(D=l.parameters)==null?void 0:D.docs,source:{originalSource:"{}",...(f=(C=l.parameters)==null?void 0:C.docs)==null?void 0:f.source}}};var H,w,P;i.parameters={...i.parameters,docs:{...(H=i.parameters)==null?void 0:H.docs,source:{originalSource:`{
  args: {
    thread: CHAT_CONFIG_THREAD.threads[CHAT_CONFIG_THREAD.current_thread_id]!.thread
  }
}`,...(P=(w=i.parameters)==null?void 0:w.docs)==null?void 0:P.source}}};var S,T,B;c.parameters={...c.parameters,docs:{...(S=c.parameters)==null?void 0:S.docs,source:{originalSource:`{
  args: {
    config: {
      host: "ide",
      lspPort: 8001,
      themeProps: {},
      features: {
        vecdb: true
      }
    }
  },
  parameters: {
    msw: {
      handlers: [goodCaps, goodPing, goodPrompts, goodUser, chatLinks, noTools]
    }
  }
}`,...(B=(T=c.parameters)==null?void 0:T.docs)==null?void 0:B.source}}};var E,A,U;m.parameters={...m.parameters,docs:{...(E=m.parameters)==null?void 0:E.docs,source:{originalSource:`{
  args: {
    thread: CHAT_WITH_KNOWLEDGE_TOOL,
    config: {
      host: "ide",
      lspPort: 8001,
      themeProps: {},
      features: {
        vecdb: true
      }
    }
  },
  parameters: {
    msw: {
      handlers: [goodCaps, goodPing, goodPrompts, goodUser,
      // noChatLinks,
      chatLinks, noTools]
    }
  }
}`,...(U=(A=m.parameters)==null?void 0:A.docs)==null?void 0:U.source}}};var b,k,y;d.parameters={...d.parameters,docs:{...(b=d.parameters)==null?void 0:b.docs,source:{originalSource:`{
  args: {
    thread: {
      id: "test",
      model: "gpt-4o",
      // or any model from STUB CAPS REQUEst
      messages: [{
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }
      // { role: "assistant", content: "👋" },
      ],
      new_chat_suggested: {
        wasSuggested: false
      }
    }
  },
  parameters: {
    msw: {
      handlers: [goodCaps, goodPing, goodPrompts, goodUser,
      // noChatLinks,
      chatLinks, noTools]
    }
  }
}`,...(y=(k=d.parameters)==null?void 0:k.docs)==null?void 0:y.source}}};var L,v,x;u.parameters={...u.parameters,docs:{...(L=u.parameters)==null?void 0:L.docs,source:{originalSource:`{
  args: {
    thread: {
      id: "test",
      model: "gpt-4o",
      // or any model from STUB CAPS REQUEst
      messages: [{
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }, {
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }, {
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }, {
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }],
      new_chat_suggested: {
        wasSuggested: false
      }
    }
  },
  parameters: {
    msw: {
      handlers: [goodCaps, goodPing, goodPrompts, goodUser,
      // noChatLinks,
      chatLinks, noTools]
    }
  }
}`,...(x=(v=u.parameters)==null?void 0:v.docs)==null?void 0:x.source}}};var O,I,j;p.parameters={...p.parameters,docs:{...(O=p.parameters)==null?void 0:O.docs,source:{originalSource:`{
  args: {
    thread: {
      id: "test",
      model: "gpt-4o",
      // or any model from STUB CAPS REQUEst
      messages: [{
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }, {
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }, {
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋"
      }, {
        role: "assistant",
        content: "👋"
      }, {
        role: "user",
        content: "Hello"
      }, {
        role: "assistant",
        content: "Hi"
      }, {
        role: "user",
        content: "👋",
        // change this to see different button colours
        compression_strength: "low"
      }, {
        role: "assistant",
        content: "👋"
      }],
      new_chat_suggested: {
        wasSuggested: false
      }
    }
  },
  parameters: {
    msw: {
      handlers: [goodCaps, goodPing, goodPrompts, goodUser,
      // noChatLinks,
      chatLinks, noTools]
    }
  }
}`,...(j=(I=p.parameters)==null?void 0:I.docs)==null?void 0:j.source}}};const Ne=["Primary","Configuration","IDE","Knowledge","EmptySpaceAtBottom","UserMessageEmptySpaceAtBottom","CompressButton"];export{p as CompressButton,i as Configuration,d as EmptySpaceAtBottom,c as IDE,m as Knowledge,l as Primary,u as UserMessageEmptySpaceAtBottom,Ne as __namedExportsOrder,Ge as default};
