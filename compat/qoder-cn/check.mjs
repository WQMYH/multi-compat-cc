import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { KEY, MODELS, startFixture } from './fixture.mjs';

const observations = [];
const fixture = await startFixture({log:event => observations.push(event)});
const headers = {'authorization':`Bearer ${KEY}`,'content-type':'application/json'};
const privateMarker = 'PRIVATE_SYNTHETIC_CONTENT_MUST_NOT_BE_LOGGED';
const send = body => fetch(`${fixture.baseUrl}/chat/completions`,{method:'POST',headers,body:JSON.stringify(body),signal:AbortSignal.timeout(5000)});
const tools = [{type:'function',function:{name:'fixture_echo',parameters:{type:'object',required:['message'],properties:{message:{type:'string'}}}}}];
const messages = [{role:'user',content:privateMarker}];
async function streamed(body) {
  const response = await send({...body,stream:true});
  assert.equal(response.status,200);
  assert.match(response.headers.get('content-type'),/^text\/event-stream/);
  const times = [];
  let wire = '';
  for await (const bytes of response.body) { times.push(performance.now()); wire += new TextDecoder().decode(bytes); }
  const data = wire.split('\n\n').filter(Boolean).map(frame => { assert.ok(frame.startsWith('data: ')); return frame.slice(6); });
  assert.equal(data.pop(),'[DONE]');
  const chunks = data.map(JSON.parse);
  assert.equal(chunks.length,4);
  assert.ok(chunks.every(chunk => chunk.object === 'chat.completion.chunk' && chunk.model === body.model));
  assert.equal(chunks[0].choices[0].delta.role,'assistant');
  assert.ok(times.length >= 3 && times.at(-1)-times[0] >= 100,'SSE must arrive incrementally');
  return chunks;
}
const roundtrip = call => send({model:MODELS[2],messages:[...messages,{role:'assistant',content:null,tool_calls:[call]},{role:'tool',tool_call_id:call.id,content:privateMarker}],tools});

try {
  const catalog = await fetch(`${fixture.baseUrl}/models`,{headers,signal:AbortSignal.timeout(5000)});
  assert.equal(catalog.status,200);
  assert.deepEqual((await catalog.json()).data.map(model => model.id),MODELS);
  const text = await send({model:MODELS[0],messages});
  assert.equal(text.status,200);
  assert.equal((await text.json()).choices[0].message.content,'FIXTURE_TEXT_OK');
  const chunks = await streamed({model:MODELS[1],messages});
  assert.equal(chunks.map(chunk => chunk.choices[0].delta.content ?? '').join(''),'FIXTURE_SSE_OK');
  assert.equal(chunks.at(-1).choices[0].finish_reason,'stop');

  const first = await send({model:MODELS[2],messages,tools});
  assert.equal(first.status,200);
  const completion = await first.json();
  assert.equal(completion.choices[0].finish_reason,'tool_calls');
  const call = completion.choices[0].message.tool_calls[0];
  assert.equal(call.type,'function');
  assert.equal(call.function.name,'fixture_echo');
  assert.deepEqual(JSON.parse(call.function.arguments),{message:'fixture-safe'});
  const result = await roundtrip(call);
  assert.equal(result.status,200);
  assert.equal((await result.json()).choices[0].message.content,'FIXTURE_TOOL_RESULT_RECEIVED');
  assert.equal((await roundtrip(call)).status,422,'Consumed tool result is not replayed');

  const toolChunks = await streamed({model:MODELS[2],messages,tools});
  assert.equal(toolChunks.at(-1).choices[0].finish_reason,'tool_calls');
  const deltas = toolChunks.flatMap(chunk => chunk.choices[0].delta.tool_calls ?? []);
  const streamedCall = {id:deltas[0].id,type:'function',function:{name:deltas[0].function.name,arguments:deltas.map(delta => delta.function.arguments).join('')}};
  const streamedResult = await roundtrip(streamedCall);
  assert.equal(streamedResult.status,200);
  assert.equal((await streamedResult.json()).choices[0].message.content,'FIXTURE_TOOL_RESULT_RECEIVED');

  const invalidAuth = await fetch(`${fixture.baseUrl}/models`,{headers:{authorization:'Bearer PRIVATE_SYNTHETIC_KEY'},signal:AbortSignal.timeout(5000)});
  assert.equal(invalidAuth.status,401);
  assert.equal((await send({model:MODELS[2],messages,tools:[]})).status,422);
  assert.equal((await send({model:MODELS[1],messages,stream:false})).status,422);
  assert.equal((await send({model:'unsupported',messages})).status,400);
  const other = await fetch(`${fixture.baseUrl}/responses`,{method:'POST',headers,body:'{}',signal:AbortSignal.timeout(5000)});
  assert.equal(other.status,404);
  const malformed = await fetch(`${fixture.baseUrl}/chat/completions`,{method:'POST',headers,body:'{bad',signal:AbortSignal.timeout(5000)});
  assert.equal(malformed.status,400);
  const oversized = await fetch(`${fixture.baseUrl}/chat/completions`,{method:'POST',headers,body:'x'.repeat(1024*1024+1),signal:AbortSignal.timeout(5000)});
  assert.equal(oversized.status,413);
  const serialized = JSON.stringify(observations);
  assert.ok(!serialized.includes(privateMarker));
  assert.ok(!serialized.includes('PRIVATE_SYNTHETIC_KEY'));
  assert.ok(!serialized.includes(KEY));
  assert.equal(observations.filter(event => event.outcome === 'tool_result_received').length,2);
  assert.ok(observations.filter(event => event.stream && event.status === 200).every(event => event.sseFrames === 4 && event.doneSent));
  console.log(JSON.stringify({result:'PASS',node:process.version,scope:'local synthetic offline only',requests:observations.length,
    checks:['model catalog','text JSON','incremental text SSE','JSON tool call/result','incremental tool SSE/result','no replay','auth rejection','unsupported route/model','malformed/oversized body','content/key redaction']}));
} finally {
  await fixture.close();
  await assert.rejects(fetch(`${fixture.baseUrl}/models`,{headers,signal:AbortSignal.timeout(2000)}));
  console.log(JSON.stringify({fixtureClosed:true}));
}

// Check the actual CLI entry point as well; own the child PID for reliable Windows cleanup.
const child = spawn(process.execPath,[fileURLToPath(new URL('./fixture.mjs',import.meta.url))],{stdio:['ignore','pipe','pipe'],windowsHide:true});
const exited = once(child,'exit');
const startupDeadline = setTimeout(() => child.kill(),5000);
let started;
try {
  let output = '';
  for await (const bytes of child.stdout.iterator({destroyOnReturn:false})) {
    output += bytes.toString();
    if (!output.includes('\n')) continue;
    started = JSON.parse(output.split('\n')[0]);
    break;
  }
  assert.equal(started.event,'started');
  assert.equal(started.processId,child.pid);
  const response = await fetch(`${started.baseUrl}/models`,{headers,signal:AbortSignal.timeout(5000)});
  assert.equal(response.status,200);
} finally {
  clearTimeout(startupDeadline);
  child.kill();
  await exited;
}
await assert.rejects(fetch(`${started.baseUrl}/models`,{headers,signal:AbortSignal.timeout(2000)}));
console.log(JSON.stringify({cliStarted:true,cliChildExited:true,cliPortClosed:true}));
