// Real native API + actual Dna.ask handoffs. No scripted successful responses.
import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import { randomUUID } from 'node:crypto';
import { startTaskService, nativeTaskEnvironmentPresent } from './native-task-harness.mjs';
import { isDescribe, isWrite, wireLine } from './command-wire.mjs';
test.skip(!nativeTaskEnvironmentPresent(),'Supply matching native API and actual-handoff seed');
const policy=(application_id,name)=>({format:'dna.task-authority/1',application_id,owner:'operations',members:['alex','blair'],grants:[{mode:'local',name,reassign:true,retire:true,recover:true}]});
const panel=page=>page.getByRole('region',{name:'Person administration',exact:true});
const recovery=page=>page.getByRole('region',{name:'Person retirement request',exact:true});
// The page carries this launch's session cookie (GH #989), again after a restart.
const open=async(page,s)=>{await s.attach(page);return page.goto(s.origin+'/#/tasks?'+new URLSearchParams({app:s.application,assignee:'alex'}));};
async function prepare(page){await panel(page).getByLabel('Retirement successor',{exact:true}).selectOption('blair');await panel(page).getByRole('button',{name:'Review retirement',exact:true}).click();}
function command(s,plan){return {request_id:randomUUID(),operation:'dna.person.retire',operation_version:'1',context:{application_id:s.application,position_id:'org'},target:{application_id:s.application,kind:'dna.person',id:'alex'},preconditions:{subject_digest:plan.subject_digest,principal:s.principal},arguments:{to:'blair'}};}
test('native retirement transfers the complete reviewed work atomically and remains inspectable',async({page},info)=>{
  const s=await startTaskService({taskPolicy:policy}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  try{
    const capabilities=await s.read(s.prefix+'/capabilities');expect(capabilities.status).toBe(200);fs.writeFileSync(s.evidence+'/person-capabilities-response.json',JSON.stringify(capabilities.json,null,2));
    const before=s.journal();const plan=await s.read(s.prefix+'/dna/people?id=alex');expect(plan.status).toBe(200);expect(plan.json.data.tasks).toHaveLength(2);expect(s.journal().head).toBe(before.head);
    const invalid=wireLine(command(s,plan.json.data));invalid.payload.to=5;const malformed=await s.post(invalid);expect(malformed.status).toBe(400);expect(malformed.code).toBe('malformed');expect(s.journal().head).toBe(before.head);
    const original=await s.current();expect((await s.post(s.command(original,'blair',randomUUID()))).status).toBe(200);const afterMove=s.journal();const stale=await s.post(command(s,plan.json.data));expect(stale.status).toBe(200);expect(stale.code).toBe('stale_subject');expect(s.journal().head).toBe(afterMove.head);
    await open(page,s);await expect(page.locator('.read-only')).toContainText('People & Task actions enabled');await expect(panel(page)).toContainText('1 open responsibilities');await prepare(page);await page.screenshot({path:info.outputPath('native-retirement-plan.png')});
    await panel(page).getByRole('button',{name:'Confirm retirement',exact:true}).click();await expect(recovery(page)).toHaveAttribute('data-observation','observed');await expect(panel(page)).toContainText('Retirement recorded');
    const final=s.journal(),added=final.rows.slice(afterMove.rows.length);expect(added.map(r=>r.kind)).toEqual(['task.reassigned','person.retired']);expect(added[0].entity).not.toBe(s.task);const fact=JSON.parse(added[1].body);expect(fact.transferred).toBe(1);expect(fact.manifest).toHaveLength(1);expect(JSON.parse(added[0].body)).toEqual({from:'alex',to:'blair',by:s.principal.name});
    const current=await s.read(s.prefix+'/dna/people?id=alex');expect(current.json.data).toMatchObject({state:'retired',successor:'blair',authorized:false,tasks:[]});
    const tasks=await s.read(s.prefix+'/dna/tasks?assignee=blair');expect(tasks.json.data.items).toHaveLength(2);for(const t of tasks.json.data.items){expect(t.acceptance_bound).toBe(true);expect(t.acceptance_digest).toBe('');expect(t.state).toBe('handed');}
    fs.writeFileSync(s.evidence+'/person-plan-response.json',JSON.stringify(plan.json,null,2));fs.writeFileSync(s.evidence+'/person-current-response.json',JSON.stringify(current.json,null,2));
    await page.setViewportSize({width:390,height:844});await page.emulateMedia({reducedMotion:'reduce'});await expect(panel(page)).toContainText('Retirement recorded');expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await page.screenshot({path:info.outputPath('native-retirement-current-mobile.png')});expect(errors).toEqual([]);
  }finally{await s.stop();await info.attach('native-retirement-service',{path:s.evidence+'/service.json',contentType:'application/json'});}
});
// Revoked: the policy's grants and the board seat both; raising work stays
// any authenticated peer's.
test('native retirement lost response survives API restart and revoked write grant with GET only',async({page})=>{
  const s=await startTaskService({taskPolicy:policy}),posts=[],errors=[];page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>{if(isWrite(r))posts.push(r.url());});
  try{
    let delivered;
    await page.route('**/commands',async route=>{if(route.request().method()!=='POST'||isDescribe(route.request()))return route.continue();const response=await route.fetch();expect(response.status()).toBe(200);delivered=await response.json();await route.abort('failed');});
    await open(page,s);await expect(page.locator('.read-only')).toContainText('People & Task actions enabled');await prepare(page);await panel(page).getByRole('button',{name:'Confirm retirement',exact:true}).click();await expect(recovery(page)).toContainText('could not be verified');expect(posts).toHaveLength(1);const admitted=s.journal();expect(admitted.rows.filter(r=>r.kind==='person.retired'&&r.entity==='alex')).toHaveLength(1);
    const service=JSON.parse(fs.readFileSync(s.evidence+'/service.json'));const updated=policy(s.application,s.principal.name);updated.grants[0].retire=false;updated.grants[0].reassign=false;fs.writeFileSync(service.policy,JSON.stringify(updated));s.unseat();await s.restart();
    await page.reload();await expect(recovery(page)).toHaveAttribute('data-observation','observed');await expect(page.locator('.read-only')).toHaveText('New tasks enabled');expect(posts).toHaveLength(1);expect(s.journal().head).toBe(admitted.head);expect(s.requests.filter(r=>r.method==='POST'&&r.path.endsWith('/commands'))).toHaveLength(0);
    fs.writeFileSync(s.evidence+'/person-command-response.json',JSON.stringify(delivered,null,2));expect(errors).toEqual([]);
  }finally{await s.stop();}
});
