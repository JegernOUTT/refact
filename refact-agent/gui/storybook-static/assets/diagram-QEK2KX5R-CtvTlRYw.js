import{E as k,_ as l,s as R,g as E,o as F,p as I,b as _,c as D,l as G,F as y,G as C,v as P,K as z,a1 as V}from"./mermaid.core-CyzbYOI-.js";import{p as W}from"./chunk-4BX2VUAB-zDmK-sNn.js";import{p as B}from"./mermaid-parser.core-ClDFW_OY.js";import"./iframe-C5HNkGZg.js";import"../sb-preview/runtime.js";import"./_commonjsHelpers-BosuxZz1.js";import"./Theme-BmxElaIf.js";import"./jsx-runtime-CKrituN3.js";import"./index-CBqU2yxZ.js";/* empty css                   */import"./v4-CQkTLCs1.js";import"./react-icons.esm-Dxg5JPe5.js";import"./get-margin-styles-oNnhyulp.js";import"./extends-CF3RwP-h.js";import"./index-BtM5VmRH.js";import"./select.module-DZDgHAIL.js";import"./index-CZwtOYEC.js";import"./index-DW5OYE7F.js";import"./theme-Bj_5e_Fh.js";import"./index-BAnWuuZQ.js";import"./index-zLxn_asd.js";import"./extract-props-H8xDRupP.js";import"./require-react-element-BKSviGtP.js";import"./index-BWbEzEeM.js";import"./index-Da_Rp7sG.js";import"./index-DiU2Igp9.js";import"./icons-CpZLwehs.js";import"./flex-DhkSe5Wr.js";import"./tooltip-DnsKDLSp.js";import"./button-CG-dCU1M.js";import"./text-field-DFysKD6w.js";import"./heading-DzHf5iNZ.js";import"./scroll-area-d_jmEDiA.js";import"./get-subtree-uFnELUcI.js";import"./box-ZI7Prd5t.js";import"./ScrollArea-DDXlGPNE.js";import"./container-DUjqJaxJ.js";import"./Checkbox-We8AotrQ.js";import"./index-Do7Jnu2F.js";import"./AnimatedText-CY2kIZkr.js";import"./index-BgSubfv6.js";import"./index-D3ylJrlI.js";import"./index-DrFu-skq.js";import"./_baseUniq-DxUG7rk9.js";import"./_basePickBy-D9suTM1Y.js";import"./clone-DcAsKeZ4.js";var h={showLegend:!0,ticks:5,max:null,min:0,graticule:"circle"},w={axes:[],curves:[],options:h},g=structuredClone(w),H=k.radar,j=l(()=>y({...H,...C().radar}),"getConfig"),b=l(()=>g.axes,"getAxes"),K=l(()=>g.curves,"getCurves"),N=l(()=>g.options,"getOptions"),U=l(r=>{g.axes=r.map(t=>({name:t.name,label:t.label??t.name}))},"setAxes"),X=l(r=>{g.curves=r.map(t=>({name:t.name,label:t.label??t.name,entries:Y(t.entries)}))},"setCurves"),Y=l(r=>{if(r[0].axis==null)return r.map(e=>e.value);const t=b();if(t.length===0)throw new Error("Axes must be populated before curves for reference entries");return t.map(e=>{const a=r.find(o=>{var i;return((i=o.axis)==null?void 0:i.$refText)===e.name});if(a===void 0)throw new Error("Missing entry for axis "+e.label);return a.value})},"computeCurveEntries"),Z=l(r=>{var e,a,o,i,n;const t=r.reduce((s,c)=>(s[c.name]=c,s),{});g.options={showLegend:((e=t.showLegend)==null?void 0:e.value)??h.showLegend,ticks:((a=t.ticks)==null?void 0:a.value)??h.ticks,max:((o=t.max)==null?void 0:o.value)??h.max,min:((i=t.min)==null?void 0:i.value)??h.min,graticule:((n=t.graticule)==null?void 0:n.value)??h.graticule}},"setOptions"),q=l(()=>{P(),g=structuredClone(w)},"clear"),$={getAxes:b,getCurves:K,getOptions:N,setAxes:U,setCurves:X,setOptions:Z,getConfig:j,clear:q,setAccTitle:R,getAccTitle:E,setDiagramTitle:F,getDiagramTitle:I,getAccDescription:_,setAccDescription:D},J=l(r=>{W(r,$);const{axes:t,curves:e,options:a}=r;$.setAxes(t),$.setCurves(e),$.setOptions(a)},"populate"),Q={parse:l(async r=>{const t=await B("radar",r);G.debug(t),J(t)},"parse")},tt=l((r,t,e,a)=>{const o=a.db,i=o.getAxes(),n=o.getCurves(),s=o.getOptions(),c=o.getConfig(),p=o.getDiagramTitle(),d=z(t),m=et(d,c),u=s.max??Math.max(...n.map(f=>Math.max(...f.entries))),x=s.min,v=Math.min(c.width,c.height)/2;rt(m,i,v,s.ticks,s.graticule),at(m,i,v,c),M(m,i,n,x,u,s.graticule,c),T(m,n,s.showLegend,c),m.append("text").attr("class","radarTitle").text(p).attr("x",0).attr("y",-c.height/2-c.marginTop)},"draw"),et=l((r,t)=>{const e=t.width+t.marginLeft+t.marginRight,a=t.height+t.marginTop+t.marginBottom,o={x:t.marginLeft+t.width/2,y:t.marginTop+t.height/2};return r.attr("viewbox",`0 0 ${e} ${a}`).attr("width",e).attr("height",a),r.append("g").attr("transform",`translate(${o.x}, ${o.y})`)},"drawFrame"),rt=l((r,t,e,a,o)=>{if(o==="circle")for(let i=0;i<a;i++){const n=e*(i+1)/a;r.append("circle").attr("r",n).attr("class","radarGraticule")}else if(o==="polygon"){const i=t.length;for(let n=0;n<a;n++){const s=e*(n+1)/a,c=t.map((p,d)=>{const m=2*d*Math.PI/i-Math.PI/2,u=s*Math.cos(m),x=s*Math.sin(m);return`${u},${x}`}).join(" ");r.append("polygon").attr("points",c).attr("class","radarGraticule")}}},"drawGraticule"),at=l((r,t,e,a)=>{const o=t.length;for(let i=0;i<o;i++){const n=t[i].label,s=2*i*Math.PI/o-Math.PI/2;r.append("line").attr("x1",0).attr("y1",0).attr("x2",e*a.axisScaleFactor*Math.cos(s)).attr("y2",e*a.axisScaleFactor*Math.sin(s)).attr("class","radarAxisLine"),r.append("text").text(n).attr("x",e*a.axisLabelFactor*Math.cos(s)).attr("y",e*a.axisLabelFactor*Math.sin(s)).attr("class","radarAxisLabel")}},"drawAxes");function M(r,t,e,a,o,i,n){const s=t.length,c=Math.min(n.width,n.height)/2;e.forEach((p,d)=>{if(p.entries.length!==s)return;const m=p.entries.map((u,x)=>{const v=2*Math.PI*x/s-Math.PI/2,f=A(u,a,o,c),O=f*Math.cos(v),S=f*Math.sin(v);return{x:O,y:S}});i==="circle"?r.append("path").attr("d",L(m,n.curveTension)).attr("class",`radarCurve-${d}`):i==="polygon"&&r.append("polygon").attr("points",m.map(u=>`${u.x},${u.y}`).join(" ")).attr("class",`radarCurve-${d}`)})}l(M,"drawCurves");function A(r,t,e,a){const o=Math.min(Math.max(r,t),e);return a*(o-t)/(e-t)}l(A,"relativeRadius");function L(r,t){const e=r.length;let a=`M${r[0].x},${r[0].y}`;for(let o=0;o<e;o++){const i=r[(o-1+e)%e],n=r[o],s=r[(o+1)%e],c=r[(o+2)%e],p={x:n.x+(s.x-i.x)*t,y:n.y+(s.y-i.y)*t},d={x:s.x-(c.x-n.x)*t,y:s.y-(c.y-n.y)*t};a+=` C${p.x},${p.y} ${d.x},${d.y} ${s.x},${s.y}`}return`${a} Z`}l(L,"closedRoundCurve");function T(r,t,e,a){if(!e)return;const o=(a.width/2+a.marginRight)*3/4,i=-(a.height/2+a.marginTop)*3/4,n=20;t.forEach((s,c)=>{const p=r.append("g").attr("transform",`translate(${o}, ${i+c*n})`);p.append("rect").attr("width",12).attr("height",12).attr("class",`radarLegendBox-${c}`),p.append("text").attr("x",16).attr("y",0).attr("class","radarLegendText").text(s.label)})}l(T,"drawLegend");var ot={draw:tt},st=l((r,t)=>{let e="";for(let a=0;a<r.THEME_COLOR_LIMIT;a++){const o=r[`cScale${a}`];e+=`
		.radarCurve-${a} {
			color: ${o};
			fill: ${o};
			fill-opacity: ${t.curveOpacity};
			stroke: ${o};
			stroke-width: ${t.curveStrokeWidth};
		}
		.radarLegendBox-${a} {
			fill: ${o};
			fill-opacity: ${t.curveOpacity};
			stroke: ${o};
		}
		`}return e},"genIndexStyles"),it=l(r=>{const t=V(),e=C(),a=y(t,e.themeVariables),o=y(a.radar,r);return{themeVariables:a,radarOptions:o}},"buildRadarStyleOptions"),nt=l(({radar:r}={})=>{const{themeVariables:t,radarOptions:e}=it(r);return`
	.radarTitle {
		font-size: ${t.fontSize};
		color: ${t.titleColor};
		dominant-baseline: hanging;
		text-anchor: middle;
	}
	.radarAxisLine {
		stroke: ${e.axisColor};
		stroke-width: ${e.axisStrokeWidth};
	}
	.radarAxisLabel {
		dominant-baseline: middle;
		text-anchor: middle;
		font-size: ${e.axisLabelFontSize}px;
		color: ${e.axisColor};
	}
	.radarGraticule {
		fill: ${e.graticuleColor};
		fill-opacity: ${e.graticuleOpacity};
		stroke: ${e.graticuleColor};
		stroke-width: ${e.graticuleStrokeWidth};
	}
	.radarLegendText {
		text-anchor: start;
		font-size: ${e.legendFontSize}px;
		dominant-baseline: hanging;
	}
	${st(t,e)}
	`},"styles"),te={parser:Q,db:$,renderer:ot,styles:nt};export{te as diagram};
