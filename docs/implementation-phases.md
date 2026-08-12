# Localhost — Implementation Phases

## Purpose

This document defines the implementation roadmap for the Localhost HTTP server.

The project must be implemented in Rust and must satisfy the project specification and audit checklist.

The implementation should proceed incrementally. Each phase must be completed, tested, and reviewed before the next phase begins.

The primary implementation agent must not skip phases or implement future-phase functionality prematurely.

---

# Global Constraints

These constraints apply to the entire project.

## Architecture

* Rust implementation.
* One process.
* One server thread.
* One central I/O multiplexing mechanism.
* Use `epoll` or an equivalent mechanism.
* Do not use `tokio`, `nix`, or crates that implement the server/networking architecture.
* `libc` may be used for required system calls.
* Unsafe code must be minimized and isolated.
* All network I/O must be non-blocking.
* All client reads and writes must be driven by the I/O multiplexer.
* The server must not crash because of malformed clients, malformed requests, configuration errors, or ordinary runtime failures.
* Resources must be cleaned up deterministically.
* No memory leaks.
* A failure affecting one client must not bring down the server.

## Event-loop invariant

There must be exactly one central event loop and one I/O multiplexer instance.

The implementation must avoid:

* per-client threads;
* blocking read loops;
* blocking write loops;
* one `select`/`poll`/`epoll` instance per client;
* background networking threads.

A single event dispatch may perform at most one read operation and at most one write operation for a client.

Partial reads and writes must be represented in connection state.

## HTTP target

The server must support HTTP/1.1.

Required methods:

* GET
* POST
* DELETE

The server must support persistent HTTP/1.1 connections.

Full concurrent HTTP/1.1 pipelining is not required.

However, the parser must correctly handle multiple HTTP requests arriving in the same TCP read and must not discard bytes belonging to subsequent requests.

Requests are processed serially per connection:

```text
Request 1
    ↓
Response 1
    ↓
Request 2
    ↓
Response 2
```

## Configuration

Use TOML for the configuration format unless the official project specification explicitly requires another syntax.

TOML is responsible for syntax/deserialization.

The application remains responsible for semantic validation.

Configuration should support:

* hosts;
* one or more ports;
* server names;
* custom error pages;
* client body limits;
* routes;
* accepted methods;
* redirects;
* filesystem roots;
* default/index files;
* directory listing;
* CGI mappings.

## Sessions

Sessions should be stored in an in-memory `HashMap`.

The single server thread owns the session store, so synchronization primitives are unnecessary.

Sessions must support:

* secure unpredictable session IDs;
* cookie-based identification;
* server-side state;
* expiration;
* cleanup without a background server thread.

## Symlinks

A filesystem path requested through a route must not escape the route's configured root through symlinks.

Symlinks resolving inside the configured root may be served.

Symlinks resolving outside the configured root must be rejected with `403 Forbidden`.

Containment must be checked using proper filesystem path semantics, not naive string-prefix matching.

---

# Phase 0 — Requirements and Repository Analysis

## Objective

Understand the project before implementing anything.

## Instructions

Read:

* project specification;
* audit checklist;
* existing repository;
* existing source files;
* build configuration;
* available test infrastructure.

Extract:

* functional requirements;
* architectural constraints;
* audit requirements;
* edge cases;
* forbidden dependencies;
* expected command-line/build behavior.

Create or update:

```text
docs/
    architecture.md
    requirements.md
    audit-checklist.md
```

Do not implement server functionality yet.

## Deliverable

A written architecture and requirements analysis.

## Completion criteria

The agent can explain:

* how the server will use epoll;
* how clients are represented;
* how configuration becomes runtime state;
* how HTTP connection state will work;
* how routing will work;
* how CGI will interact with the server;
* how timeouts will work;
* how sessions will work.

---

# Phase 1 — Architecture and Data Model

## Objective

Define the internal architecture before networking implementation.

## Instructions

Design explicit structures for:

* server configuration;
* listeners;
* client connections;
* connection state;
* HTTP parser state;
* HTTP request;
* HTTP response;
* routes;
* sessions;
* CGI processes;
* timeouts.

The architecture must separate:

```text
Configuration
      ↓
Networking
      ↓
HTTP parsing
      ↓
Routing
      ↓
Request handling
      ↓
Response generation
      ↓
Networking
```

Avoid coupling HTTP parsing directly to sockets.

Avoid coupling configuration parsing directly to socket creation.

Avoid global mutable state unless strictly justified.

## Deliverable

`docs/architecture.md`

Include:

* component diagram;
* connection state machine;
* request lifecycle;
* response lifecycle;
* ownership model;
* error-handling strategy.

## Completion criteria

The architecture can support all later phases without requiring a redesign of the event loop.

---

# Phase 2 — Networking Foundation

## Objective

Implement the single-threaded non-blocking event loop.

## Implement

* listening sockets;
* multiple listeners;
* non-blocking sockets;
* one epoll instance;
* central event loop;
* accepting clients;
* client registration;
* client removal;
* disconnect handling;
* socket error handling.

## Do NOT implement

* HTTP parsing;
* routing;
* CGI;
* cookies;
* sessions;
* static file serving.

## Mandatory invariants

1. Exactly one epoll instance.
2. Exactly one server thread.
3. Exactly one central event loop.
4. All network sockets are non-blocking.
5. Reads occur only after readable events.
6. Writes occur only after writable events.
7. At most one read per client per event dispatch.
8. At most one write per client per event dispatch.
9. Partial I/O is represented in connection state.
10. Fatal socket errors remove the client.
11. Client disconnects are handled correctly.
12. One client cannot terminate the server.

## Tests

Test:

* multiple simultaneous clients;
* connect/disconnect cycles;
* clients closing unexpectedly;
* EAGAIN/EWOULDBLOCK;
* EINTR;
* invalid socket states;
* repeated connections.

---

# Phase 3 — Configuration and Validation

## Objective

Implement configuration loading and semantic validation.

## Format

Use TOML.

Recommended architecture:

```text
config.toml
    ↓
TOML deserialization
    ↓
Config structs
    ↓
semantic validation
    ↓
validated runtime configuration
```

## Configuration must support

* single server;
* multiple servers;
* multiple ports;
* multiple server names;
* custom error pages;
* body-size limits;
* routes;
* route methods;
* redirects;
* filesystem roots;
* default/index files;
* directory listing;
* CGI mappings.

## Validation

Detect:

* malformed TOML;
* missing required fields;
* invalid ports;
* invalid methods;
* duplicate/conflicting listeners;
* invalid body limits;
* invalid route configuration;
* invalid CGI configuration where detectable;
* impossible server configurations.

Configuration errors must not crash the server.

## Audit tests

Create fixtures for:

```text
single_server.toml
multiple_ports.toml
virtual_hosts.toml
custom_errors.toml
body_limit.toml
routes.toml
methods.toml
duplicate_ports.toml
invalid.toml
```

---

# Phase 4 — HTTP/1.1 Parser and Connection State Machine

## Objective

Implement robust HTTP request parsing independent of application logic.

## Support

* request line;
* HTTP version;
* headers;
* header validation;
* Content-Length;
* Transfer-Encoding;
* Host;
* Connection;
* cookies;
* request body;
* fragmented requests;
* multiple requests in one read.

## Parser requirements

TCP boundaries must not be treated as HTTP message boundaries.

The parser must correctly handle:

```text
one request = many reads
many requests = one read
partial headers
partial body
```

Maintain an input buffer per connection.

Never discard unprocessed bytes.

## HTTP/1.1

Support persistent connections.

Process requests serially.

Do not implement concurrent pipelined request processing.

If multiple complete requests exist in the buffer, preserve them for sequential processing.

## Error handling

Generate appropriate responses for malformed requests.

At minimum support:

* 400;
* 404;
* 405;
* 413;
* 500.

---

# Phase 5 — Response Engine and Static File Serving

## Objective

Serve static content correctly.

## Implement

* HTTP response construction;
* status lines;
* response headers;
* Content-Length;
* Content-Type;
* Connection;
* Date where appropriate;
* partial writes;
* static files;
* directories;
* default/index files;
* configured error pages.

## Filesystem safety

Normalize URL paths.

Prevent:

```text
../
```

path traversal.

Prevent symlink escapes from the configured route root.

Return:

* 403 for forbidden access;
* 404 for missing resources.

## MIME types

Implement a reasonable extension-to-content-type mapping.

Unknown types should receive a safe generic content type.

## Response buffering

Do not perform blocking writes.

Represent partial responses explicitly:

```text
response buffer
write offset
remaining bytes
```

Register writable interest only when necessary.

---

# Phase 6 — Routing and Virtual Hosts

## Objective

Implement route selection and server selection.

## Virtual host selection

Given:

```text
local address
local port
Host header
```

select the appropriate configured server.

If no `server_name` matches, use the first/default server for that host:port.

## Routes

Routes must support:

* path matching;
* filesystem root;
* accepted methods;
* redirect;
* default/index file;
* directory listing;
* CGI mapping.

Routes do not require regular expressions.

## Method handling

If a route exists but the method is not allowed:

```text
405 Method Not Allowed
```

Include an appropriate `Allow` header where applicable.

## Audit tests

Test:

```text
single server
multiple ports
multiple hostnames
curl --resolve
default server
route matching
method restrictions
redirects
directory defaults
```

---

# Phase 7 — Request Bodies, POST, Chunked Encoding, and Uploads

## Objective

Implement request-body handling robustly.

## Support

* Content-Length requests;
* chunked requests;
* body-size limits;
* POST;
* file uploads;
* fragmented bodies.

## Content-Length

Validate:

* numeric syntax;
* overflow;
* body limit;
* consistency with received data.

Do not allocate unbounded memory based on client-controlled values.

## Chunked encoding

Implement:

```text
chunk-size
chunk-data
CRLF
...
0
trailers
```

Correctly handle:

* chunk boundaries split across reads;
* chunk size split across reads;
* empty chunks;
* final zero chunk;
* trailers;
* malformed chunk syntax;
* body limits.

## Uploads

Store uploaded content safely.

Verify uploaded files are not corrupted.

## 413

If the configured body limit is exceeded:

```text
413 Payload Too Large
```

The connection must remain in a well-defined state or be closed safely according to the request-processing strategy.

---

# Phase 8 — DELETE, Cookies, and Sessions

## DELETE

Implement DELETE for configured routes/files.

Handle:

* successful deletion;
* missing files;
* forbidden paths;
* directories where deletion is not allowed;
* filesystem errors.

Return correct HTTP status codes.

## Cookies

Parse the Cookie request header.

Generate `Set-Cookie` responses.

Support:

* session ID;
* HttpOnly;
* Path;
* appropriate expiration behavior.

## Sessions

Use:

```rust
HashMap<SessionId, Session>
```

owned by the main server thread.

A session should contain at minimum:

* session ID;
* creation time;
* last access time;
* application data.

## Expiration

Expire stale sessions.

Do not introduce a cleanup thread.

Perform cleanup from the main event loop.

---

# Phase 9 — CGI

## Objective

Implement one CGI type.

Example:

```text
.py → Python CGI
```

## Requirements

The CGI:

* executes as a child process;
* receives the file to process as its first argument;
* receives EOF after the request body;
* receives `PATH_INFO`;
* executes from the correct working directory;
* receives relevant CGI environment variables.

## Important

CGI execution must not block the main server event loop.

The server must be able to continue servicing other clients while a CGI process is running.

Implement appropriate process tracking and timeout handling.

## Failure cases

Handle:

* executable missing;
* fork failure;
* exec failure;
* CGI exits abnormally;
* CGI timeout;
* malformed CGI response;
* oversized CGI output where applicable.

Return:

```text
500 Internal Server Error
```

when appropriate.

---

# Phase 10 — Timeouts, Resource Management, and Hardening

## Objective

Ensure the server cannot be trivially exhausted by slow or malicious clients.

## Implement timeouts for

* incomplete request headers;
* incomplete request bodies;
* slow uploads;
* slow response consumers;
* CGI processes;
* idle persistent connections where appropriate.

## Resource limits

Review:

* maximum header size;
* maximum request line;
* maximum number of headers;
* maximum body size;
* maximum response size where applicable;
* maximum simultaneous clients;
* CGI process limits.

## Required property

A client that stops sending data must not permanently occupy a connection.

A client that stops reading responses must not permanently block the server.

---

# Phase 11 — Browser Compatibility and Integration Testing

## Objective

Test with an actual browser.

Use the browser's developer tools.

## Test

* static HTML;
* CSS;
* JavaScript;
* images;
* favicon;
* multiple resources;
* persistent connections;
* redirects;
* 404;
* directory listing;
* directory index;
* POST;
* CGI;
* cookies;
* sessions.

## Inspect

Request headers:

* Host;
* Connection;
* Cookie;
* Content-Length;
* Transfer-Encoding.

Response headers:

* Content-Type;
* Content-Length;
* Connection;
* Set-Cookie;
* Location;
* status code.

The server must successfully serve a complete static website.

---

# Phase 12 — Automated Integration Test Suite

## Objective

Create exhaustive tests that map directly to the audit checklist.

Organize tests by category:

```text
tests/
├── configuration/
├── networking/
├── http/
├── routing/
├── methods/
├── uploads/
├── cookies/
├── sessions/
├── cgi/
├── errors/
├── browser/
└── stress/
```

## Required test categories

### Networking

* multiple clients;
* connect/disconnect;
* malformed clients;
* partial reads;
* partial writes;
* client reset.

### Configuration

* single server;
* multiple ports;
* multiple hostnames;
* custom errors;
* body limits;
* routes;
* index files;
* method restrictions;
* duplicate ports;
* invalid configuration.

### HTTP

* valid GET;
* valid POST;
* valid DELETE;
* malformed request;
* malformed headers;
* missing Host;
* incorrect Content-Length;
* chunked body;
* fragmented request;
* multiple requests in one read;
* persistent connection.

### Filesystem

* normal file;
* missing file;
* forbidden file;
* directory;
* index;
* directory listing;
* traversal attempt;
* symlink inside root;
* symlink outside root.

### CGI

* valid CGI;
* POST body;
* chunked POST;
* unchunked POST;
* PATH_INFO;
* relative paths;
* CGI failure;
* CGI timeout.

### Sessions

* session creation;
* session reuse;
* cookie parsing;
* expiration;
* multiple clients.

---

# Phase 13 — Stress, Leak, and Final Audit

## Objective

Verify that the server remains available under stress and does not leak resources.

## Siege

Run:

```text
siege -b [IP]:[PORT]
```

Target:

```text
availability >= 99.5%
```

## Stress scenarios

Test:

* many clients;
* repeated connections;
* persistent connections;
* malformed requests;
* large requests;
* slow clients;
* CGI requests;
* uploads.

## Memory

Use appropriate tools to detect:

* memory leaks;
* file descriptor leaks;
* zombie processes;
* growing connection state;
* growing session state.

## Long-running test

Run the server for an extended period while repeatedly:

* connecting;
* disconnecting;
* requesting files;
* sending malformed requests;
* uploading;
* invoking CGI.

Verify resource usage remains stable.

---

# Final Audit Checklist

Before submission, verify every item in `audit-checklist.md`.

## Functional

* [ ] HTTP server architecture can be explained.
* [ ] I/O multiplexing mechanism can be explained.
* [ ] Exactly one central multiplexer is used.
* [ ] Exactly one server thread is used.
* [ ] One read/write per client per event dispatch.
* [ ] I/O return values are checked.
* [ ] Fatal socket errors remove clients.
* [ ] All network I/O goes through the multiplexer.
* [ ] No blocking network I/O exists.

## Configuration

* [ ] Single server works.
* [ ] Multiple ports work.
* [ ] Multiple hostnames work.
* [ ] Host-based virtual server selection works.
* [ ] Custom error pages work.
* [ ] Body limits work.
* [ ] Routes work.
* [ ] Default/index files work.
* [ ] Method restrictions work.
* [ ] Configuration errors are detected.

## Methods

* [ ] GET works.
* [ ] POST works.
* [ ] DELETE works.
* [ ] Wrong requests do not crash the server.
* [ ] Uploads work.
* [ ] Uploaded files are not corrupted.
* [ ] Cookies work.
* [ ] Sessions work.

## Browser

* [ ] Browser connects successfully.
* [ ] Static website works.
* [ ] Request headers are correct.
* [ ] Response headers are correct.
* [ ] Wrong URLs are handled.
* [ ] Directory listing works.
* [ ] Redirects work.
* [ ] CGI works with chunked data.
* [ ] CGI works with unchunked data.

## Ports

* [ ] Multiple ports work.
* [ ] Multiple websites work.
* [ ] Duplicate port configuration is detected.
* [ ] One invalid server configuration does not unnecessarily destroy valid independent configurations.

## Stress

* [ ] Siege availability >= 99.5%.
* [ ] No memory leak.
* [ ] No file descriptor leak.
* [ ] No zombie CGI processes.
* [ ] No hanging connections.
* [ ] Server survives malformed clients.
* [ ] Server survives client disconnects.

---

# Agent Operating Rules

The implementation agent must follow these rules throughout the project.

## 1. Do not skip phases

Do not implement Phase N+1 functionality while Phase N is incomplete unless explicitly instructed.

## 2. Verify before claiming success

Never claim:

* "non-blocking";
* "no memory leak";
* "HTTP compliant";
* "browser compatible";
* "no crashes";

without performing appropriate verification.

## 3. Preserve architecture

Do not introduce:

* additional server threads;
* per-client threads;
* blocking network operations;
* Tokio;
* Nix;
* hidden networking abstractions.

## 4. Keep unsafe code minimal

Every unsafe block must have a clear reason and safety invariant.

## 5. Test after each phase

At minimum:

```text
cargo check
cargo test
```

Run relevant integration tests after functional phases.

## 6. Review after each major phase

Use a fresh review context to inspect:

* event-loop correctness;
* HTTP correctness;
* resource management;
* security;
* configuration;
* CGI.

The reviewer should identify issues rather than independently redesigning the entire project.

## 7. Do not silently change requirements

If implementation details conflict with the project specification or audit checklist:

1. stop;
2. identify the conflict;
3. explain the options;
4. choose the option that best satisfies the explicit requirements.

## 8. Keep an audit trail

For each phase record:

```text
Phase:
Implemented:
Files changed:
Tests added:
Tests executed:
Known limitations:
Audit requirements satisfied:
Remaining risks:
```

---

# Definition of Done

The project is complete only when:

1. All phases have been implemented.
2. All automated tests pass.
3. Browser testing passes.
4. CGI testing passes.
5. Siege availability reaches the required threshold.
6. Resource usage remains stable during extended testing.
7. No known file descriptor or process leaks remain.
8. Every audit question has a demonstrated answer.
9. The implementation can be explained from source code.
10. The server remains stable when presented with malformed or hostile client behavior.
11. The implementation respects the one-process/one-thread/one-multiplexer architecture.
12. No forbidden networking/server crates are used.
