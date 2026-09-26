// Actual Task filter/native command proof. Organization member navigation is
// covered separately by scripted source reads; this seed has no invented map.
import { test, expect } from '@playwright/test';
import {startTaskService,nativeTaskEnvironmentPresent} from './native-task-harness.mjs';
test.skip(true, "The HTTP record-command route was cut (GH #1104 piece 5, PR #1129): record commands are the head socket's gated topics, which a browser cannot reach; this lane waits for the face's write path.");
test.skip(!nativeTaskEnvironmentPresent(),'Supply matching native Task API and accepted actual-handoff seed');
test('exact assignee list follows native reassignment while the same Task detail stays available',async({page},info)=>{
 const service=await startTaskService({taskPolicy:(application_id,name)=>({format:'dna.task-authority/1',application_id,owner:'operations',members:['alex','blair'],grants:[{mode:'local',name,reassign:true,recover:true}]})});
 const errors=[],posts=[],gets=[];page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>{if(r.method()==='POST')posts.push(r.url());if(r.method()==='GET'&&r.url().includes('/dna/tasks'))gets.push(new URL(r.url()));});
 try {
  await page.goto(service.origin+'/#/tasks?'+new URLSearchParams({app:service.application,assignee:'alex',id:service.task}));
  const list=page.getByRole('region',{name:'Handed Tasks',exact:true}),detail=page.getByRole('region',{name:'Handed Task administration',exact:true});
  await expect(list.getByRole('region',{name:'Assignments for alex',exact:true})).toContainText('recorded assignee');
  await expect(list.locator('.record-list > li')).toHaveCount(2);await expect(detail).toBeVisible();expect(posts).toHaveLength(0);
  await detail.getByLabel('New assignee',{exact:true}).selectOption('blair');await detail.getByRole('button',{name:'Review reassignment',exact:true}).click();
  await page.getByRole('group',{name:'Confirm Task reassignment',exact:true}).getByRole('button',{name:'Confirm reassignment',exact:true}).click();
  await expect(page.getByRole('region',{name:'Task reassignment request',exact:true})).toHaveAttribute('data-observation','observed');
  await expect(detail.locator('.task-current-assignment')).toContainText('blair');await expect(page.getByRole('region',{name:'Task responsibility',exact:true})).toContainText('This Task is now recorded under blair. The list still shows assignments for alex.');
  await expect(list.locator('.record-list > li')).toHaveCount(1);expect((await service.current()).assignee).toBe('blair');expect(posts).toHaveLength(1);
  expect(gets.some(u=>u.searchParams.get('assignee')==='alex'&&!u.searchParams.has('id'))).toBe(true);expect(gets.some(u=>u.searchParams.get('id')===service.task&&!u.searchParams.has('assignee'))).toBe(true);
  await page.screenshot({path:info.outputPath('actual-assignee-detail-after-reassignment.png')});
  await page.getByRole('link',{name:'Back to handed Tasks',exact:true}).click();await expect(list.locator('.record-list > li')).toHaveCount(1);expect(new URLSearchParams(new URL(page.url()).hash.split('?')[1]).get('assignee')).toBe('alex');
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({reducedMotion:'reduce'});await expect(list.getByRole('region',{name:'Assignments for alex',exact:true})).toBeVisible();await expect(list.locator('.record-list > li')).toHaveCount(1);expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.screenshot({path:info.outputPath('actual-assignee-list-narrow.png')});
  await list.getByRole('link',{name:'All assignees',exact:true}).click();await expect(list.locator('.record-list > li')).toHaveCount(2);expect(posts).toHaveLength(1);expect(errors).toEqual([]);
 } finally {await service.stop();await info.attach('native-task-assignee-service',{path:service.evidence+'/service.json',contentType:'application/json'});}
});
