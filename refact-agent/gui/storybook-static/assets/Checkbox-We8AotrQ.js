import{j as h}from"./jsx-runtime-CKrituN3.js";import{t as j,e as w}from"./flex-DhkSe5Wr.js";import{r as t}from"./index-CBqU2yxZ.js";import{a as P,$ as E,r as V,i as I,c as N}from"./get-margin-styles-oNnhyulp.js";import{_ as $}from"./extends-CF3RwP-h.js";import{$ as W,d as k,c as B}from"./index-CZwtOYEC.js";import{$ as D,a as M}from"./index-DW5OYE7F.js";import{$ as O}from"./index-DiU2Igp9.js";import{v as A}from"./extract-props-H8xDRupP.js";import{r as K}from"./icons-CpZLwehs.js";const R="Checkbox",[L,ie]=W(R),[H,T]=L(R),X=t.forwardRef((e,l)=>{const{__scopeCheckbox:a,name:o,checked:c,defaultChecked:n,required:i,disabled:d,value:m="on",onCheckedChange:b,...x}=e,[s,v]=t.useState(null),q=P(l,r=>v(r)),S=t.useRef(!1),C=s?!!s.closest("form"):!0,[u=!1,g]=D({prop:c,defaultProp:n,onChange:b}),z=t.useRef(u);return t.useEffect(()=>{const r=s==null?void 0:s.form;if(r){const f=()=>g(z.current);return r.addEventListener("reset",f),()=>r.removeEventListener("reset",f)}},[s,g]),t.createElement(H,{scope:a,state:u,disabled:d},t.createElement(E.button,$({type:"button",role:"checkbox","aria-checked":p(u)?"mixed":u,"aria-required":i,"data-state":_(u),"data-disabled":d?"":void 0,disabled:d,value:m},x,{ref:q,onKeyDown:k(e.onKeyDown,r=>{r.key==="Enter"&&r.preventDefault()}),onClick:k(e.onClick,r=>{g(f=>p(f)?!0:!f),C&&(S.current=r.isPropagationStopped(),S.current||r.stopPropagation())})})),C&&t.createElement(J,{control:s,bubbles:!S.current,name:o,value:m,checked:u,required:i,disabled:d,style:{transform:"translateX(-100%)"}}))}),F="CheckboxIndicator",G=t.forwardRef((e,l)=>{const{__scopeCheckbox:a,forceMount:o,...c}=e,n=T(F,a);return t.createElement(B,{present:o||p(n.state)||n.state===!0},t.createElement(E.span,$({"data-state":_(n.state),"data-disabled":n.disabled?"":void 0},c,{ref:l,style:{pointerEvents:"none",...e.style}})))}),J=e=>{const{control:l,checked:a,bubbles:o=!0,...c}=e,n=t.useRef(null),i=O(a),d=M(l);return t.useEffect(()=>{const m=n.current,b=window.HTMLInputElement.prototype,s=Object.getOwnPropertyDescriptor(b,"checked").set;if(i!==a&&s){const v=new Event("click",{bubbles:o});m.indeterminate=p(a),s.call(m,p(a)?!1:a),m.dispatchEvent(v)}},[i,a,o]),t.createElement("input",$({type:"checkbox","aria-hidden":!0,defaultChecked:p(a)?!1:a},c,{tabIndex:-1,ref:n,style:{...e.style,...d,position:"absolute",pointerEvents:"none",opacity:0,margin:0}}))};function p(e){return e==="indeterminate"}function _(e){return p(e)?"indeterminate":e?"checked":"unchecked"}const Q=X,Y=G,U=t.forwardRef((e,l)=>{const{className:a,color:o,...c}=A(e,I,V);return t.createElement(Q,{"data-accent-color":o,...c,asChild:!1,ref:l,className:N("rt-reset","rt-BaseCheckboxRoot","rt-CheckboxRoot",a)},t.createElement(Y,{asChild:!0,className:"rt-BaseCheckboxIndicator rt-CheckboxIndicator"},t.createElement(K,null)))});U.displayName="Checkbox";const y=({name:e,checked:l,disabled:a,onCheckedChange:o,children:c,title:n,...i})=>h.jsx(j,{as:"label",size:"2",title:n,children:h.jsxs(w,{wrap:"nowrap",gap:"2",children:[h.jsx(U,{size:"1",...i,name:e,checked:l,disabled:a,onCheckedChange:o}),c]})});try{y.displayName="Checkbox",y.__docgenInfo={description:"",displayName:"Checkbox",props:{m:{defaultValue:null,description:`Sets the CSS **margin** property.
Supports space scale values, CSS strings, and responsive objects.
@example m="4"
m="100px"
m={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin`,name:"m",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mx:{defaultValue:null,description:`Sets the CSS **margin-left** and **margin-right** properties.
Supports space scale values, CSS strings, and responsive objects.
@example mx="4"
mx="100px"
mx={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-left
https://developer.mozilla.org/en-US/docs/Web/CSS/margin-right`,name:"mx",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},my:{defaultValue:null,description:`Sets the CSS **margin-top** and **margin-bottom** properties.
Supports space scale values, CSS strings, and responsive objects.
@example my="4"
my="100px"
my={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-top
https://developer.mozilla.org/en-US/docs/Web/CSS/margin-bottom`,name:"my",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mt:{defaultValue:null,description:`Sets the CSS **margin-top** property.
Supports space scale values, CSS strings, and responsive objects.
@example mt="4"
mt="100px"
mt={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-top`,name:"mt",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mr:{defaultValue:null,description:`Sets the CSS **margin-right** property.
Supports space scale values, CSS strings, and responsive objects.
@example mr="4"
mr="100px"
mr={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-right`,name:"mr",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mb:{defaultValue:null,description:`Sets the CSS **margin-bottom** property.
Supports space scale values, CSS strings, and responsive objects.
@example mb="4"
mb="100px"
mb={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-bottom`,name:"mb",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},ml:{defaultValue:null,description:`Sets the CSS **margin-left** property.
Supports space scale values, CSS strings, and responsive objects.
@example ml="4"
ml="100px"
ml={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-left`,name:"ml",required:!1,type:{name:'Responsive<Union<string, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},highContrast:{defaultValue:null,description:"",name:"highContrast",required:!1,type:{name:"boolean"}},color:{defaultValue:null,description:"",name:"color",required:!1,type:{name:"enum",value:[{value:'"ruby"'},{value:'"gray"'},{value:'"gold"'},{value:'"bronze"'},{value:'"brown"'},{value:'"yellow"'},{value:'"amber"'},{value:'"orange"'},{value:'"tomato"'},{value:'"red"'},{value:'"crimson"'},{value:'"pink"'},{value:'"plum"'},{value:'"purple"'},{value:'"violet"'},{value:'"iris"'},{value:'"indigo"'},{value:'"blue"'},{value:'"cyan"'},{value:'"teal"'},{value:'"jade"'},{value:'"green"'},{value:'"grass"'},{value:'"lime"'},{value:'"mint"'},{value:'"sky"'}]}},size:{defaultValue:null,description:"",name:"size",required:!1,type:{name:'Responsive<"1" | "2" | "3">'}},variant:{defaultValue:null,description:"",name:"variant",required:!1,type:{name:"enum",value:[{value:'"classic"'},{value:'"surface"'},{value:'"soft"'}]}}}}}catch{}export{y as C,U as r};
