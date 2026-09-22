// Scripted browser contracts. Native atomicity and authority have separate gates.
import { test as base, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { httpFixture } from './runtime-harness.mjs';

const APP='a'.repeat(40), HEAD='b'.repeat(40), NEXT='c'.repeat(40), EVENT='d'.repeat(40), DIGEST='sha256:'+'e'.repeat(64);
const WHO='mara / équipe', TO='lee', PRINCIPAL={mode:'local',name:'riley'};
const test=base.extend({host:async({},use)=>{
  const names=['index.html','app.js','styles.css','runtime.js','application.js','organization-draft.js','definition-draft.js','knowledge-draft.js','task-administration.js', 'projects.js', 'task-create.js'];
  const assets=new Map(await Promise.all(names.map(async n=>[n,await readFile(new URL('../web/'+n,import.meta.url))])));
  const host=await httpFixture((req,res)=>{const name=req.url==='/'?'index.html':req.url.slice(1);if(!assets.has(name)){res.writeHead(404).end();return;}res.setHeader('content-type',name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':'text/html');res.end(assets.get(name));});
  try{await use(host);}finally{await host.close();}
}});
async function fixture(page,options={}){
  const script={applied:false,authorized:true,lost:false,bad:false,count:2,posts:[],gets:[],errors:[],...options};page.on('pageerror',e=>script.errors.push(e.message));
  const source=()=>({record_id:APP,record_head:script.applied?NEXT:HEAD,record_revision:script.applied?'13':'10'});
  const wrap=data=>({api_version:'hale.v1',source:source(),data});
  const rows=()=>Array.from({length:script.count},(_,i)=>({id:'task/'+i,outcome:'Carry responsibility '+i,state:'handed',assignee:script.applied?script.command.arguments.to:WHO,obligation:'handover',acceptance_digest:'',acceptance_bound:true,evidence_required:false,evidence_ref:'',waiting:'',assignment_digest:DIGEST,reassignment_supported:true,history:[{event_id:'1'.repeat(40),sequence:'3',kind:'task.handed',from:'',to:WHO,by:'org'},...(script.applied?[{event_id:(''+(i+2)).repeat(40),sequence:String(10+i),kind:'task.reassigned',from:WHO,to:script.command.arguments.to,by:PRINCIPAL.name}]:[])]}));
  const receipt=()=>wrap({command_id:'command/'+script.command.request_id,request_id:script.command.request_id,application_id:APP,operation:'dna.person.retire',operation_version:'1',principal:PRINCIPAL,context:script.command.context,target:script.command.target,subject_digest:DIGEST,fingerprint:'sha256:'+'f'.repeat(64),state:'succeeded',reason:'',person:{state:'applied',from:WHO,to:script.command.arguments.to,event_id:EVENT,transferred:String(script.count)},review:{state:'unavailable',outcome:'',subject_digest:''},activation:{state:'unknown',reason:''}});
  await page.route('**/api/hale/v1{,/**}',async route=>{
    const req=route.request(),url=new URL(req.url()),send=(status,data)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(data)});
    if(url.pathname==='/api/hale/v1/applications')return send(200,wrap({items:[{id:APP,kind:'dna',name:'People contract',capabilities_url:'/api/hale/v1/applications/'+APP+'/capabilities'}],page:{limit:25,offset:0,total:1,next_offset:-1,snapshot:source().record_head}}));
    if(url.pathname.endsWith('/capabilities'))return send(200,wrap({application_id:APP,principal:PRINCIPAL,read_only:!script.authorized,reads:{tasks:true,people:true,organization:false,workflows:false,practices:false,reviews:false,definitions:false,knowledge:false},writes:{practice_propose:false,review_verdict:false,person_retire:script.authorized},person_commands:{profile:'dna.person.retire.v1',available:true,authorized:script.authorized,position_id:'org',recovery:'record_lifetime',max_identity_bytes:'256',max_request_bytes:'32768',max_transfers:'32',recipients:script.authorized?[WHO,TO]:[],reason:''}}));
    if(url.pathname.endsWith('/commands')){
      if(req.method()==='POST'){script.command=req.postDataJSON();script.posts.push(script.command);script.applied=true;if(script.lost){script.authorized=false;return route.abort('failed');}return send(200,receipt());}
      script.gets.push(url.searchParams.get('request_id'));return send(200,receipt());
    }
    if(url.searchParams.has('snapshot')&&url.searchParams.get('snapshot')!==source().record_head)return send(409,{api_version:'hale.v1',error:{code:'snapshot_changed',message:'Record changed',retryable:true}});
    if(url.pathname.endsWith('/dna/tasks')){
      const assignee=url.searchParams.get('assignee'),id=url.searchParams.get('id'),items=rows().filter(t=>(!assignee||t.assignee===assignee)&&(!id||t.id===id));
      return send(200,wrap({profile:'dna.task-administration.v1',...(assignee?{assignee}:{}),items,page:{limit:25,offset:0,total:items.length,next_offset:-1,snapshot:source().record_head},basis:{projection:'dna.task-administration/1',memory:'record',routing:'0',record_head:source().record_head,record_revision:source().record_revision}}));
    }
    if(url.pathname.endsWith('/dna/people'))return send(200,wrap({profile:'dna.person-administration.v1',person:script.bad?'different':WHO,state:script.applied?'retired':'active',successor:script.applied?script.command.arguments.to:'',event_id:script.applied?EVENT:'',subject_digest:DIGEST,tasks:script.applied?[]:rows().map(t=>({id:t.id,assignment_digest:t.assignment_digest,from:WHO})),transferred:script.applied?'0':String(script.count),authorized:script.authorized&&!script.applied,recipients:script.authorized&&!script.applied?[WHO,TO]:[],basis:{memory:'record',routing:'0',record_head:source().record_head,record_revision:source().record_revision}}));
    return send(404,{api_version:'hale.v1',error:{code:'not_found',message:'Missing scripted route',retryable:false}});
  });return script;
}
const panel=page=>page.getByRole('region',{name:'Person administration',exact:true});
const recovery=page=>page.getByRole('region',{name:'Person retirement request',exact:true});
const open=(page,host)=>page.goto(host.origin+'/#/tasks?'+new URLSearchParams({app:APP,assignee:WHO}));
async function prepare(page,to=TO){await panel(page).getByLabel('Retirement successor',{exact:true}).selectOption(to);await panel(page).getByRole('button',{name:'Review retirement',exact:true}).click();}
test('person retirement shows the exact work, confirms once and observes the same retirement',async({page,host},info)=>{
  const s=await fixture(page);await open(page,host);await expect(panel(page)).toBeVisible();await expect(panel(page).getByRole('list',{name:'Responsibilities included in retirement'}).locator('li')).toHaveCount(2);expect(s.posts).toEqual([]);
  await prepare(page);await expect(panel(page).getByRole('group',{name:'Confirm person retirement'})).toContainText('2 open responsibilities');expect(s.posts).toEqual([]);
  await page.screenshot({path:info.outputPath('retirement-plan-desktop.png')});
  await panel(page).getByRole('button',{name:'Confirm retirement',exact:true}).click();await expect(recovery(page)).toHaveAttribute('data-observation','observed');await expect(panel(page)).toContainText('Retirement recorded');expect(s.posts).toHaveLength(1);
  expect(s.posts[0]).toMatchObject({operation:'dna.person.retire',target:{id:WHO,kind:'dna.person'},preconditions:{subject_digest:DIGEST,principal:PRINCIPAL},arguments:{to:TO}});expect(s.posts[0].preconditions).not.toHaveProperty('assignee');
  expect(s.errors).toEqual([]);
});
test('person retirement permits no successor only for the reviewed empty plan and fits narrow view',async({page,host},info)=>{
  const s=await fixture(page,{count:0});await page.setViewportSize({width:390,height:844});await page.emulateMedia({reducedMotion:'reduce'});await open(page,host);await expect(panel(page)).toBeVisible();
  await prepare(page,'');await expect(panel(page).getByRole('group',{name:'Confirm person retirement'})).toContainText('Future requests for this person remain unassigned.');
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:info.outputPath('retirement-plan-mobile.png')});
  await panel(page).getByRole('button',{name:'Keep person active'}).click();expect(s.posts).toEqual([]);expect(s.errors).toEqual([]);
});
test('person retirement clears a mismatched plan and does not infer authority from assignment',async({page,host})=>{
  const s=await fixture(page,{bad:true});await open(page,host);await expect(page.getByText('The complete retirement plan is unavailable. No retirement can be prepared from this view.')).toBeVisible();await expect(panel(page)).toHaveCount(0);expect(s.posts).toEqual([]);expect(s.errors).toEqual([]);
});
test('lost retirement reply recovers by GET after reload with write grant revoked',async({page,host})=>{
  const s=await fixture(page,{lost:true});await open(page,host);await prepare(page);await panel(page).getByRole('button',{name:'Confirm retirement',exact:true}).click();await expect(recovery(page)).toContainText('could not be verified');
  const stored=await page.evaluate(()=>Object.values(localStorage).map(v=>JSON.parse(v)));expect(stored).toHaveLength(1);expect(stored[0]).toMatchObject({version:6,operation:'dna.person.retire',target_id:WHO});expect(stored[0]).not.toHaveProperty('to');
  await page.reload();await expect(recovery(page)).toHaveAttribute('data-observation','observed');expect(s.posts).toHaveLength(1);expect(s.gets).toContain(s.posts[0].request_id);expect(s.errors).toEqual([]);
});
