import{j as e}from"./jsx-runtime-CKrituN3.js";import{R as I,r as k}from"./index-CBqU2yxZ.js";import{C as O,u as W,g as $,s as t,v as w,I as D,a as f,p as S,b as y}from"./select.module-DZDgHAIL.js";import{c as V}from"./get-margin-styles-oNnhyulp.js";import{I as E}from"./theme-Bj_5e_Fh.js";import{n as P}from"./container-DUjqJaxJ.js";import"./_commonjsHelpers-BosuxZz1.js";import"./extends-CF3RwP-h.js";import"./index-CZwtOYEC.js";import"./index-BtM5VmRH.js";import"./index-DW5OYE7F.js";import"./extract-props-H8xDRupP.js";import"./require-react-element-BKSviGtP.js";import"./index-BWbEzEeM.js";import"./index-zLxn_asd.js";import"./index-Da_Rp7sG.js";import"./index-DiU2Igp9.js";import"./index-BAnWuuZQ.js";import"./icons-CpZLwehs.js";import"./get-subtree-uFnELUcI.js";function T(l){return!l||typeof l!="object"||!("type"in l)?!1:l.type==="separator"}const d=O,p=W,c=l=>e.jsx($,{...l,className:V(t.content,l.className)}),s=l=>e.jsx(w,{...l,className:V(t.item,l.className)}),v=D,g=({title:l,options:m,onChange:q,contentPosition:z,...r})=>{const[R,U]=I.useState(r.open??r.defaultOpen??!1),o=k.useMemo(()=>{if(typeof r.value>"u")return null;const a=m.find(n=>typeof n!="string"&&"value"in n&&n.value===r.value);return!a||typeof a=="string"?null:a},[r.value,m]);return e.jsxs(d,{...r,onValueChange:q,onOpenChange:U,size:"1",children:[o&&"tooltip"in o&&o.tooltip&&!R?e.jsxs(f,{openDelay:1e3,children:[e.jsx(S,{children:e.jsx("span",{children:e.jsx(p,{})})}),e.jsx(y,{size:"1",side:"top",children:o.tooltip})]}):e.jsx(p,{title:l}),e.jsx(c,{position:z??"popper",children:m.map((a,n)=>typeof a=="string"?e.jsx(s,{value:a,children:a},`select-item-${n}-${a}`):T(a)?e.jsx(v,{},a.key??`separator-${n}`):a.tooltip?e.jsx(s,{...a,children:e.jsxs(f,{openDelay:1e3,children:[e.jsx(S,{children:e.jsxs("div",{children:[e.jsx("span",{className:t.trigger_only,children:a.textValue??a.value}),e.jsx("span",{className:t.dropdown_only,children:a.children})]})}),e.jsx(y,{size:"1",children:a.tooltip})]})},`select-item-${n}-${a.value}`):e.jsxs(s,{...a,children:[e.jsx("span",{className:t.trigger_only,children:a.textValue??a.value}),e.jsx("span",{className:t.dropdown_only,children:a.children})]},`select-item-${n}-${a.value}`))})]})};try{d.displayName="Root",d.__docgenInfo={description:"",displayName:"Root",props:{size:{defaultValue:null,description:"",name:"size",required:!1,type:{name:'Responsive<"1" | "2" | "3">'}}}}}catch{}try{p.displayName="Trigger",p.__docgenInfo={description:"",displayName:"Trigger",props:{m:{defaultValue:null,description:`Sets the CSS **margin** property.
Supports space scale values, CSS strings, and responsive objects.
@example m="4"
m="100px"
m={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin`,name:"m",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mx:{defaultValue:null,description:`Sets the CSS **margin-left** and **margin-right** properties.
Supports space scale values, CSS strings, and responsive objects.
@example mx="4"
mx="100px"
mx={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-left
https://developer.mozilla.org/en-US/docs/Web/CSS/margin-right`,name:"mx",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},my:{defaultValue:null,description:`Sets the CSS **margin-top** and **margin-bottom** properties.
Supports space scale values, CSS strings, and responsive objects.
@example my="4"
my="100px"
my={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-top
https://developer.mozilla.org/en-US/docs/Web/CSS/margin-bottom`,name:"my",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mt:{defaultValue:null,description:`Sets the CSS **margin-top** property.
Supports space scale values, CSS strings, and responsive objects.
@example mt="4"
mt="100px"
mt={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-top`,name:"mt",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mr:{defaultValue:null,description:`Sets the CSS **margin-right** property.
Supports space scale values, CSS strings, and responsive objects.
@example mr="4"
mr="100px"
mr={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-right`,name:"mr",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},mb:{defaultValue:null,description:`Sets the CSS **margin-bottom** property.
Supports space scale values, CSS strings, and responsive objects.
@example mb="4"
mb="100px"
mb={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-bottom`,name:"mb",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},ml:{defaultValue:null,description:`Sets the CSS **margin-left** property.
Supports space scale values, CSS strings, and responsive objects.
@example ml="4"
ml="100px"
ml={{ sm: '6', lg: '9' }}
@link https://developer.mozilla.org/en-US/docs/Web/CSS/margin-left`,name:"ml",required:!1,type:{name:'Responsive<Union<string, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-1" | "-2" | "-3" | "-4" | "-5" | "-6" | "-7" | "-8" | "-9">>'}},placeholder:{defaultValue:null,description:"",name:"placeholder",required:!1,type:{name:"string"}},radius:{defaultValue:null,description:"",name:"radius",required:!1,type:{name:"enum",value:[{value:'"small"'},{value:'"none"'},{value:'"medium"'},{value:'"large"'},{value:'"full"'}]}},color:{defaultValue:null,description:"",name:"color",required:!1,type:{name:"enum",value:[{value:'"ruby"'},{value:'"gray"'},{value:'"gold"'},{value:'"bronze"'},{value:'"brown"'},{value:'"yellow"'},{value:'"amber"'},{value:'"orange"'},{value:'"tomato"'},{value:'"red"'},{value:'"crimson"'},{value:'"pink"'},{value:'"plum"'},{value:'"purple"'},{value:'"violet"'},{value:'"iris"'},{value:'"indigo"'},{value:'"blue"'},{value:'"cyan"'},{value:'"teal"'},{value:'"jade"'},{value:'"green"'},{value:'"grass"'},{value:'"lime"'},{value:'"mint"'},{value:'"sky"'}]}},variant:{defaultValue:null,description:"",name:"variant",required:!1,type:{name:"enum",value:[{value:'"classic"'},{value:'"surface"'},{value:'"soft"'},{value:'"ghost"'}]}}}}}catch{}try{c.displayName="Content",c.__docgenInfo={description:"",displayName:"Content",props:{highContrast:{defaultValue:null,description:"",name:"highContrast",required:!1,type:{name:"boolean"}},color:{defaultValue:null,description:"",name:"color",required:!1,type:{name:"enum",value:[{value:'"ruby"'},{value:'"gray"'},{value:'"gold"'},{value:'"bronze"'},{value:'"brown"'},{value:'"yellow"'},{value:'"amber"'},{value:'"orange"'},{value:'"tomato"'},{value:'"red"'},{value:'"crimson"'},{value:'"pink"'},{value:'"plum"'},{value:'"purple"'},{value:'"violet"'},{value:'"iris"'},{value:'"indigo"'},{value:'"blue"'},{value:'"cyan"'},{value:'"teal"'},{value:'"jade"'},{value:'"green"'},{value:'"grass"'},{value:'"lime"'},{value:'"mint"'},{value:'"sky"'}]}},variant:{defaultValue:null,description:"",name:"variant",required:!1,type:{name:"enum",value:[{value:'"soft"'},{value:'"solid"'}]}}}}}catch{}try{s.displayName="Item",s.__docgenInfo={description:"",displayName:"Item",props:{tooltip:{defaultValue:null,description:"",name:"tooltip",required:!1,type:{name:"ReactNode"}}}}}catch{}try{v.displayName="Separator",v.__docgenInfo={description:"",displayName:"Separator",props:{}}}catch{}try{g.displayName="Select",g.__docgenInfo={description:"",displayName:"Select",props:{size:{defaultValue:null,description:"",name:"size",required:!1,type:{name:'Responsive<"1" | "2" | "3">'}},onChange:{defaultValue:null,description:"",name:"onChange",required:!0,type:{name:"(value: string) => void"}},options:{defaultValue:null,description:"",name:"options",required:!0,type:{name:"(string | ItemProps | SeparatorOption)[]"}},title:{defaultValue:null,description:"",name:"title",required:!1,type:{name:"string"}},contentPosition:{defaultValue:null,description:"",name:"contentPosition",required:!1,type:{name:"enum",value:[{value:'"item-aligned"'},{value:'"popper"'}]}}}}}catch{}const oe={title:"Select",component:g,decorators:[l=>e.jsx(E,{children:e.jsx(P,{children:e.jsx(l,{})})})]},N="long".repeat(30),i={args:{options:["apple","banana","orange",N],onChange:()=>({}),defaultValue:"apple"}},u={args:{options:[{value:"apple"},{value:"banana",disabled:!0},{value:"orange"},{value:N}],onChange:()=>({}),defaultValue:"apple"}};var h,_,x;i.parameters={...i.parameters,docs:{...(h=i.parameters)==null?void 0:h.docs,source:{originalSource:`{
  args: {
    options: ["apple", "banana", "orange", long],
    onChange: () => ({}),
    defaultValue: "apple"
  }
}`,...(x=(_=i.parameters)==null?void 0:_.docs)==null?void 0:x.source}}};var b,j,C;u.parameters={...u.parameters,docs:{...(b=u.parameters)==null?void 0:b.docs,source:{originalSource:`{
  args: {
    options: [{
      value: "apple"
    }, {
      value: "banana",
      disabled: true
    }, {
      value: "orange"
    }, {
      value: long
    }],
    onChange: () => ({}),
    defaultValue: "apple"
  }
}`,...(C=(j=u.parameters)==null?void 0:j.docs)==null?void 0:C.source}}};const ie=["Default","OptionObject"];export{i as Default,u as OptionObject,ie as __namedExportsOrder,oe as default};
