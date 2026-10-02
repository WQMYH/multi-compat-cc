import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { pathToFileURL } from 'node:url';

export const KEY = 'qoder-fixture-only';
export const MODELS = ['qoder-fixture-text', 'qoder-fixture-sse', 'qoder-fixture-tool'];
const MAX_BODY = 1024 * 1024;
const safeName = value => typeof value === 'string' && /^[A-Za-z0-9_.:-]{1,80}$/.test(value) ? value : '[omitted]';
const schemaSummary = tool => ({
  name: safeName(tool?.function?.name),
  required: (Array.isArray(tool?.function?.parameters?.required) ? tool.function.parameters.required : []).slice(0,16).map(safeName),
  properties: Object.entries(tool?.function?.parameters?.properties ?? {}).slice(0,16).map(([name, value]) => ({name:safeName(name), type:safeName(value?.type)})),
});

export async function startFixture({log = event => console.log(JSON.stringify(event)), toolName = 'fixture_echo', toolArgs = {message:'fixture-safe'}} = {}) {
  if (safeName(toolName) === '[omitted]' || !toolArgs || typeof toolArgs !== 'object' || Array.isArray(toolArgs)) throw new Error('Invalid fixture tool configuration');
  const argumentsText = JSON.stringify(toolArgs);
  if (Buffer.byteLength(argumentsText) > 4096) throw new Error('Fixture tool arguments exceed 4096 bytes');
  let sequence = 0;
  const pending = new Map();
  const server = http.createServer(async (req, res) => {
    const id = ++sequence;
    const route = ['/v1/models','/v1/chat/completions'].includes(req.url) ? req.url : '[unsupported]';
    const event = {event:'request', id, method:['GET','POST'].includes(req.method) ? req.method : '[other]', route,
      authPresent:typeof req.headers.authorization === 'string', authMatchesFixture:req.headers.authorization === `Bearer ${KEY}`};
    const record = value => { if (id <= 201) log(value); };
    const json = (status, body) => { res.writeHead(status, {'content-type':'application/json'}); res.end(JSON.stringify(body)); };
    const reject = (status, code) => { record({...event, status, outcome:code}); json(status, {error:{message:code,type:'fixture_error',code}}); };
    try {
      // ponytail: 200 requests per run and 16 pending calls; restart for another bounded probe.
      if (id > 200) return reject(429,'fixture_request_limit');
      if (!event.authMatchesFixture) return reject(401,'fixture_key_required');
      if (req.method === 'GET' && route === '/v1/models') {
        record({...event,status:200,outcome:'model_list'});
        return json(200,{object:'list',data:MODELS.map(model => ({id:model,object:'model',created:0,owned_by:'local-fixture'}))});
      }
      if (req.method !== 'POST' || route !== '/v1/chat/completions') return reject(404,'unsupported_route_no_replay');
      if (req.headers['content-encoding'] && req.headers['content-encoding'] !== 'identity') return reject(415,'unsupported_content_encoding');
      const parts = [];
      let size = 0;
      for await (const part of req) {
        size += part.length;
        if (size > MAX_BODY) { reject(413,'fixture_body_limit'); res.on('finish', () => req.destroy()); return; }
        parts.push(part);
      }
      let body;
      try { body = JSON.parse(Buffer.concat(parts).toString('utf8')); } catch { return reject(400,'invalid_json'); }
      if (!body || typeof body !== 'object' || Array.isArray(body) || !MODELS.includes(body.model) ||
          !Array.isArray(body.messages) || body.messages.length < 1 || body.messages.length > 128 ||
          body.messages.some(message => !message || !['system','developer','user','assistant','tool'].includes(message.role)) ||
          (body.stream !== undefined && typeof body.stream !== 'boolean') ||
          (body.tools !== undefined && (!Array.isArray(body.tools) || body.tools.length > 64))) return reject(400,'unsupported_request_shape');
      Object.assign(event,{model:body.model,stream:body.stream === true,bodyBytes:size,
        messageCount:body.messages.length,roles:body.messages.map(message => message.role),
        tools:(body.tools ?? []).slice(0,16).map(schemaSummary)});
      if (body.model === 'qoder-fixture-sse' && body.stream !== true) return reject(422,'sse_model_requires_stream_true');
      let message = {role:'assistant',content:'FIXTURE_TEXT_OK'};
      let finishReason = 'stop';
      let outcome = 'text_sent';
      if (body.model === 'qoder-fixture-sse') { message.content = 'FIXTURE_SSE_OK'; outcome = 'sse_sent'; }
      if (body.model === 'qoder-fixture-tool') {
        const last = body.messages.at(-1);
        if (last.role === 'tool') {
          const call = pending.get(last.tool_call_id);
          const previous = body.messages.at(-2);
          const echoed = previous?.role === 'assistant' && previous.tool_calls?.some(value => value.id === last.tool_call_id && value.type === 'function' && value.function?.name === toolName && value.function?.arguments === argumentsText);
          if (!call || !echoed || typeof last.content !== 'string') return reject(422,'unmatched_tool_result_no_replay');
          pending.delete(last.tool_call_id);
          event.toolResultMatched = true;
          event.toolResultChars = last.content.length;
          message.content = 'FIXTURE_TOOL_RESULT_RECEIVED';
          outcome = 'tool_result_received';
        } else {
          if (!(body.tools ?? []).some(tool => tool.type === 'function' && tool.function?.name === toolName)) return reject(422,'configured_tool_not_offered');
          if (pending.size >= 16) return reject(429,'fixture_pending_call_limit');
          const callId = `call_fixture_${randomUUID()}`;
          pending.set(callId,true);
          message = {role:'assistant',content:null,tool_calls:[{id:callId,type:'function',function:{name:toolName,arguments:argumentsText}}]};
          finishReason = 'tool_calls';
          outcome = 'tool_call_sent';
          event.toolName = toolName;
        }
      }
      const base = {id:`chatcmpl-fixture-${id}`,created:Math.floor(Date.now()/1000),model:body.model};
      if (!event.stream) {
        record({...event,status:200,outcome});
        return json(200,{...base,object:'chat.completion',choices:[{index:0,message,finish_reason:finishReason}],usage:{prompt_tokens:0,completion_tokens:0,total_tokens:0}});
      }
      res.writeHead(200,{'content-type':'text/event-stream; charset=utf-8','cache-control':'no-cache','connection':'keep-alive'});
      res.flushHeaders();
      let frames = 0;
      const send = (delta, finish = null) => { if (res.destroyed) return; res.write(`data: ${JSON.stringify({...base,object:'chat.completion.chunk',choices:[{index:0,delta,finish_reason:finish}]})}\n\n`); frames++; };
      send({role:'assistant',content:''});
      if (message.tool_calls) {
        const call = message.tool_calls[0];
        const split = Math.ceil(argumentsText.length/2);
        await delay(80);
        send({tool_calls:[{index:0,id:call.id,type:'function',function:{name:toolName,arguments:argumentsText.slice(0,split)}}]});
        await delay(80);
        send({tool_calls:[{index:0,function:{arguments:argumentsText.slice(split)}}]});
      } else {
        const split = Math.ceil(message.content.length/2);
        await delay(80); send({content:message.content.slice(0,split)});
        await delay(80); send({content:message.content.slice(split)});
      }
      send({},finishReason);
      if (!res.destroyed) { res.end('data: [DONE]\n\n'); record({...event,status:200,outcome,sseFrames:frames,doneSent:true}); }
      else record({...event,status:200,outcome:'client_disconnected',sseFrames:frames,doneSent:false});
    } catch { if (!res.headersSent) reject(400,'request_read_failed'); else res.destroy(); }
  });
  server.requestTimeout = 10000;
  server.headersTimeout = 10000;
  await new Promise((resolve,reject) => { server.once('error',reject); server.listen(0,'127.0.0.1',resolve); });
  return {server,baseUrl:`http://127.0.0.1:${server.address().port}/v1`,close:() => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); })};
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const flags = process.argv.slice(2);
  if (flags.some(flag => !flag.startsWith('--tool-name=') && !flag.startsWith('--tool-args='))) throw new Error('Use --tool-name=NAME --tool-args=JSON only');
  const toolName = flags.find(flag => flag.startsWith('--tool-name='))?.slice(12);
  const rawArgs = flags.find(flag => flag.startsWith('--tool-args='))?.slice(12);
  const fixture = await startFixture({toolName,toolArgs:rawArgs === undefined ? undefined : JSON.parse(rawArgs)});
  console.log(JSON.stringify({event:'started',processId:process.pid,baseUrl:fixture.baseUrl,apiKey:KEY,models:MODELS,toolName:toolName ?? 'fixture_echo',expiresInSeconds:900}));
  const stop = async () => { clearTimeout(expiry); await fixture.close(); console.log(JSON.stringify({event:'stopped'})); };
  const expiry = setTimeout(stop,900000);
  process.once('SIGINT',stop);
  process.once('SIGTERM',stop);
}
