# Localhost — Audit Checklist

Preparation checklist covering the six required audit sections. Each item is a thing to be able to **demonstrate in running code** and **justify verbally**. Bonus/General section omitted.

---

## 1. Functional — server internals

Be ready to open the source and point at the exact lines for each of these.

- [x] **Explain how an HTTP server works** — bind/listen/accept loop, parsing the request line and headers, routing to a handler, building and writing the response, connection lifecycle.
- [x] **Name the I/O multiplexing function used and explain it** — `select` / `poll` / `epoll` / `kqueue`. Explain what it monitors, what it returns, and why blocking I/O per-client would not scale.
- [x] **Show there is exactly one select/poll call** serving both reads and writes for all clients, not one loop for reading and a separate one for writing.
- [x] **Explain why a single select matters and how it was achieved** — one authoritative view of readiness, no starvation, no blocking on a socket that isn't ready. Point to the single event loop.
- [x] **Trace the path from select to a client read/write** — show that there is at most one `read` and one `write` per client per select iteration, not a loop draining the socket.
- [x] **Show return values of every I/O call are checked** — `read`, `write`, `accept`, `recv`, `send`. Handle `0` (peer closed) distinctly from `-1` (error).
- [x] **Show that an error on a socket removes the client** — closes the fd, removes it from the fd set / poll array, frees any per-client state.
- [x] **Confirm no read or write ever happens outside the select loop** — including error pages, CGI output, and file serving.

---

## 2. Configuration file

Have a config prepared that demonstrates each of these, and be ready to edit it live.

- [x] **Single server, single port** — starts, serves, responds.
- [x] **Multiple servers on different ports** — both reachable simultaneously.
- [x] **Multiple servers on different hostnames, same IP:port** — verify with:
  ```bash
  curl --resolve test.com:80:127.0.0.1 http://test.com/
  curl --resolve other.com:80:127.0.0.1 http://other.com/
  ```
  Different content must come back. Be ready to explain the `Host` header's role.
- [x] **Custom error pages** — trigger a 404 and a 403 and show the configured page is served, not a built-in default.
- [x] **Client body size limit** — verify both under and over the limit:
  ```bash
  curl -X POST -H "Content-Type: text/plain" --data "short" http://localhost:8080/
  curl -X POST -H "Content-Type: text/plain" --data "<oversized payload>" http://localhost:8080/
  ```
  Oversized should return `413`.
- [x] **Routes are honoured** — a configured route resolves to its configured root/alias, not the server default.
- [x] **Default file for directory requests** — requesting `/somedir/` serves the configured index file.
- [ ] **Per-route method whitelist** — `DELETE` succeeds on a route that permits it and returns `405` on one that doesn't.

---

## 3. Methods and cookies

Check the **status code** on every one of these, not just the body.

- [x] **GET works** — `200` on an existing file, `404` on a missing one.
- [x] **POST works** — correct status, body received and handled.
- [x] **DELETE works** — resource actually removed, correct status, `404` on repeat.
- [x] **Malformed request doesn't kill the server** — send garbage, a bad request line, or a missing `Host`; expect `400` and a server that keeps serving afterwards.
- [x] **File upload round-trip is byte-identical** — upload, download, compare:
  ```bash
  diff original.bin downloaded.bin
  ```
  Test with a binary file, not just text.
- [x] **Sessions and cookies work** — `Set-Cookie` on first response, cookie echoed back on subsequent requests, session state persists across them.

---

## 4. Interaction with the browser

Do all of this with DevTools open on the Network tab.

- [ ] **Browser connects cleanly** — no console errors, no hanging requests, no protocol warnings.
- [ ] **Request and response headers are correct** — `Content-Length` accurate, `Content-Type` correct per file type, `Connection` handled. A full static site (HTML + CSS + JS + images) loads with no broken assets.
- [ ] **Wrong URL handled** — `404` with the configured error page, not a hang or a crash.
- [ ] **Directory listing handled** — either an autoindex page or the configured refusal, per config. Show both states by toggling the setting.
- [ ] **Redirect handled** — `301`/`302` with a correct `Location`, and the browser actually follows it.
- [ ] **CGI works with chunked and unchunked data** — test both. Be ready to explain how chunked bodies are unchunked before being handed to the CGI process.

---

## 5. Port issues

This section is about **error handling**, not happy paths.

- [ ] **Multiple ports with multiple sites all work** — each port serves its own configured content.
- [ ] **Duplicate port in config is caught** — configuring the same port twice must produce a clear error at startup rather than silently binding one and dropping the other. Be ready to show where the conflict is detected.
- [ ] **One bad server block doesn't take down the rest** — with several servers sharing common ports, an invalid config in one must leave the valid ones serving. Be ready to explain *why* this is desirable behaviour and where the isolation happens in the code.

---

## 6. Siege and stress testing

- [ ] **Availability ≥ 99.5%** on an empty page:
  ```bash
  siege -b 127.0.0.1:8080
  ```
  Run long enough to be meaningful and have the output ready to show.
- [ ] **No memory leak** — watch RSS in `top` or `htop` during a sustained siege run. Memory should plateau, not climb steadily.
- [ ] **No hanging connections** — check with `lsof -i` or `ss -tan` after the siege ends. Sockets should be released, not accumulating in `CLOSE_WAIT`.

---

## Before the audit

- [ ] Config file is clean, commented, and covers every scenario above without needing edits mid-audit
- [ ] Test files ready — a binary for the upload round-trip, an oversized payload for the body limit, a static site for the browser test
- [ ] CGI scripts working and their interpreters present on the audit machine
- [ ] Server rebuilds from clean with a single command
- [ ] You can navigate to any relevant source file within a few seconds