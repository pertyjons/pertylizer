---
name: start-synth
description: Start the modular synth with MCP support so Claude can inspect and control it
disable-model-invocation: true
allowed-tools: Bash
---

Start the modular synth with MCP support enabled.

## Steps

1. Check if a synth is already running on port 9850 and kill it:
   ```bash
   PID=$(ss -tlnp | grep 9850 | grep -oP 'pid=\K\d+')
   if [ -n "$PID" ]; then
     kill "$PID"
     sleep 1
   fi
   ```

2. Build and run with MCP feature:
   ```bash
   cargo run --features mcp
   ```
   Run this in the background so Claude can continue working.

4. Wait 3 seconds, then verify the MCP server is listening:
   ```bash
   ss -tlnp | grep 9850
   ```

5. Fetch available MCP tools by connecting directly via TCP with newline-delimited JSON-RPC.
   The MCP protocol requires a handshake before any tool calls work:
   ```bash
   python3 -c "
   import socket, json, time

   s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
   s.connect(('127.0.0.1', 9850))
   s.settimeout(5)

   def send_msg(sock, msg):
       sock.sendall((json.dumps(msg) + '\n').encode())

   def recv_all(sock, timeout=2):
       sock.settimeout(timeout)
       buf = b''
       try:
           while True:
               chunk = sock.recv(8192)
               if not chunk:
                   break
               buf += chunk
       except socket.timeout:
           pass
       return buf.decode()

   # Step 1: Initialize handshake (REQUIRED before any tool calls)
   send_msg(s, {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'claude-code','version':'1.0'}}})
   time.sleep(1)
   recv_all(s, 1)

   # Step 2: Send initialized notification (completes handshake)
   send_msg(s, {'jsonrpc':'2.0','method':'notifications/initialized'})
   time.sleep(0.3)

   # Step 3: Now we can call tools/list
   send_msg(s, {'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}})
   time.sleep(1)
   resp = recv_all(s, 2)
   data = json.loads(resp)
   for tool in data['result']['tools']:
       schema = tool.get('inputSchema', {})
       props = schema.get('properties', {})
       params = ', '.join(f\"{k}: {v.get('type','?')}\" for k,v in props.items())
       print(f\"  - {tool['name']}({params})\")
       print(f\"    {tool['description'][:120]}\")
   s.close()
   "
   ```

6. Report status to the user:
   - Whether GUI started
   - Whether MCP server is listening on 127.0.0.1:9850
   - List of available MCP tools

## MCP Connection Protocol

**IMPORTANT:** When calling MCP tools later in the session, always use this same protocol:
- Connect to `127.0.0.1:9850` via TCP
- Use **newline-delimited JSON-RPC** (each message is a JSON object followed by `\n`)
- **Always perform the 3-step handshake first:** `initialize` request → read response → `notifications/initialized` notification
- Only then send `tools/call` requests
- Do NOT use Content-Length framing — the rmcp server uses plain newline-delimited JSON over TCP
