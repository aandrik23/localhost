# Localhost — Audit Checklist

This document contains the audit criteria for the Localhost HTTP server project.

The purpose of this document is not only to verify that the server works, but also to verify that the student understands the implementation and can explain the relevant source code.

The auditor may ask the student to locate and explain the implementation in the source code.

---

# 1. Functional / I/O Multiplexing

## 1.1 HTTP server fundamentals

* [ ] Student can explain how an HTTP server works.
* [ ] Student can explain the request/response model.
* [ ] Student can explain the role of TCP.
* [ ] Student can explain the relationship between the browser/client and the server.
* [ ] Student can explain the relevant parts of HTTP/1.1 used by the implementation.

## 1.2 I/O multiplexing

* [ ] Student can identify which I/O multiplexing mechanism is used.
* [ ] Student can explain how the chosen mechanism works.
* [ ] Student can identify where the multiplexing mechanism is initialized.
* [ ] Student can identify where the multiplexing call is performed.
* [ ] Student can explain how listening sockets are registered.
* [ ] Student can explain how client sockets are registered.
* [ ] Student can explain how readable events are handled.
* [ ] Student can explain how writable events are handled.
* [ ] Student can explain how socket errors are handled.

## 1.3 Single multiplexing mechanism

The server must use a single central I/O multiplexing mechanism for client communication.

* [ ] There is one central event loop.
* [ ] There is one central epoll/select/poll mechanism.
* [ ] The server does not create a separate event loop for each client.
* [ ] The server does not create a thread per client.
* [ ] The server does not use blocking client I/O outside the event loop.

## 1.4 One read/write per client per event

The auditor specifically checks how the code moves from the multiplexing call to client I/O.

For each client returned by the multiplexing mechanism:

* [ ] At most one read/recv operation is performed for that client during that event.
* [ ] At most one write/send operation is performed for that client during that event.
* [ ] The implementation does not loop on read until `EAGAIN`.
* [ ] The implementation does not loop on write until the response is completely sent.
* [ ] Partial reads are stored in connection state.
* [ ] Partial writes are stored in connection state.
* [ ] The socket is allowed to return to the multiplexing mechanism before another I/O operation is attempted.

The student must be able to explain:

* Why one read/write per event is used.
* How partial reads are handled.
* How partial writes are handled.
* Why the server does not block while processing a client.

## 1.5 I/O return values

Every network I/O operation must have its return value checked.

Verify:

* [ ] `read`/`recv` return values are checked.
* [ ] `write`/`send` return values are checked.
* [ ] `accept` return values are checked.
* [ ] multiplexing function return values are checked.
* [ ] socket creation errors are checked.
* [ ] `bind` errors are checked.
* [ ] `listen` errors are checked.
* [ ] socket registration errors are checked.
* [ ] close/cleanup errors are handled appropriately.

The implementation correctly distinguishes:

* [ ] successful operations;
* [ ] `EAGAIN`;
* [ ] `EWOULDBLOCK`;
* [ ] `EINTR`;
* [ ] connection closure;
* [ ] fatal socket errors.

## 1.6 Client removal

When a client socket encounters an unrecoverable error:

* [ ] The client is removed from the multiplexing mechanism.
* [ ] The socket is closed.
* [ ] Connection state is released.
* [ ] Associated resources are released.
* [ ] Other clients continue to be served.
* [ ] A broken client cannot crash the server.

## 1.7 All I/O is event-driven

* [ ] Client reads only occur after the multiplexing mechanism indicates readability.
* [ ] Client writes only occur after the multiplexing mechanism indicates writability.
* [ ] There is no blocking client socket I/O.
* [ ] There are no hidden blocking reads.
* [ ] There are no hidden blocking writes.
* [ ] The server does not busy-loop on sockets.

---

# 2. Configuration File

The auditor may modify the configuration file and restart the server.

The server must correctly handle configuration changes.

## 2.1 Single server

* [ ] One server with one port works.
* [ ] Requests reach the correct server.
* [ ] The server responds correctly.

## 2.2 Multiple servers / different ports

* [ ] Multiple servers can listen on different ports.
* [ ] Each port reaches the correct configuration.
* [ ] Each server can have different routes/configuration.
* [ ] Requests to one port do not accidentally use another server's configuration.

## 2.3 Multiple servers / different hostnames

Test with a command such as:

```bash
curl --resolve test.com:80:127.0.0.1 http://test.com/
```

Verify:

* [ ] Multiple `server_name` values work.
* [ ] The HTTP `Host` header is used to select the correct server.
* [ ] Different hostnames can share the same IP address.
* [ ] Different hostnames can share the same port.
* [ ] The correct configuration is selected based on hostname.
* [ ] An unknown hostname uses the configured default server.

The student can explain:

* How the listening socket is identified.
* How the HTTP `Host` header is processed.
* How the final server configuration is selected.

## 2.4 Custom error pages

* [ ] Custom error pages can be configured.
* [ ] Configured error pages are actually served.
* [ ] Missing custom error pages are handled safely.
* [ ] Default error pages are available when no custom page is configured.

## 2.5 Client body-size limit

Configure a body limit and test with requests such as:

```bash
curl -X POST \
     -H "Content-Type: text/plain" \
     --data "BODY" \
     http://127.0.0.1:PORT/
```

Verify:

* [ ] Bodies smaller than the limit are accepted.
* [ ] Bodies equal to the limit behave correctly.
* [ ] Bodies larger than the limit are rejected.
* [ ] The server returns `413 Payload Too Large` when appropriate.
* [ ] The server does not allocate unbounded memory.
* [ ] Chunked requests are also subject to the body limit.

## 2.6 Routes

* [ ] Routes are parsed correctly.
* [ ] Routes are matched correctly.
* [ ] Route roots are respected.
* [ ] Different routes can have different configurations.
* [ ] Route configuration does not leak into unrelated routes.

## 2.7 Default files

* [ ] A configured default file is served when a request targets a directory.
* [ ] Missing default files are handled correctly.
* [ ] Default-file behavior works for nested directories.

## 2.8 Allowed methods

* [ ] A route can specify accepted methods.
* [ ] Allowed methods work.
* [ ] Disallowed methods return the appropriate status.
* [ ] `405 Method Not Allowed` is returned when appropriate.

Example:

```text
GET /resource
DELETE /resource
```

Verify that DELETE succeeds or fails according to route configuration.

---

# 3. Methods and Cookies

For each method, verify both successful and unsuccessful requests.

The auditor should inspect the returned status codes.

## 3.1 GET

* [ ] GET requests work.
* [ ] Existing files return the correct response.
* [ ] Missing files return `404`.
* [ ] Forbidden resources return `403`.
* [ ] Directory requests behave correctly.
* [ ] Default files work.
* [ ] Directory listing works when enabled.
* [ ] Directory listing is disabled when configured.

## 3.2 POST

* [ ] POST requests work.
* [ ] Request bodies are received correctly.
* [ ] Content-Length bodies work.
* [ ] Chunked bodies work.
* [ ] Oversized bodies are rejected.
* [ ] POST can be routed to CGI where configured.

## 3.3 DELETE

* [ ] DELETE requests work.
* [ ] Permitted resources can be deleted.
* [ ] Forbidden DELETE operations are rejected.
* [ ] Missing resources return an appropriate status.
* [ ] Route method restrictions are respected.

## 3.4 Invalid requests

Send malformed or incorrect requests.

Verify:

* [ ] The server does not crash.
* [ ] The server returns an appropriate HTTP error.
* [ ] The connection is handled correctly.
* [ ] Other clients remain unaffected.
* [ ] The server continues accepting requests afterward.

## 3.5 File uploads

* [ ] Files can be uploaded.
* [ ] Uploaded files are stored correctly.
* [ ] Uploaded files can be retrieved.
* [ ] Retrieved files match the original contents.
* [ ] Large uploads are handled correctly.
* [ ] Upload limits are enforced.
* [ ] Interrupted uploads do not corrupt server state.

## 3.6 Cookies

* [ ] Server can send `Set-Cookie`.
* [ ] Client sends cookies back using `Cookie`.
* [ ] Server correctly parses cookies.
* [ ] Multiple cookies are handled correctly.
* [ ] Invalid cookie input does not crash the server.

## 3.7 Sessions

* [ ] A session can be created.
* [ ] Session identifiers are stored in cookies.
* [ ] Existing sessions can be retrieved.
* [ ] Different clients do not accidentally share sessions.
* [ ] Session expiration/cleanup works.
* [ ] Client-controlled session identifiers are not blindly trusted as application state.

---

# 4. Browser Interaction

Use the browser selected by the team during the audit.

Open the browser developer tools and inspect network requests/responses.

## 4.1 Basic browser connectivity

* [ ] Browser connects to the server.
* [ ] Browser receives valid HTTP responses.
* [ ] Browser does not report protocol errors.
* [ ] Persistent connections behave correctly.

## 4.2 Static website

Serve a complete static website.

Verify:

* [ ] HTML works.
* [ ] CSS works.
* [ ] JavaScript works.
* [ ] Images work.
* [ ] Other static assets work.
* [ ] MIME types are appropriate.
* [ ] Favicon requests are handled correctly.

Inspect:

* [ ] Request headers.
* [ ] Response headers.
* [ ] Status codes.
* [ ] Content-Length where applicable.
* [ ] Connection behavior.
* [ ] Cookies where applicable.

## 4.3 Wrong URL

Request a nonexistent URL.

Verify:

* [ ] Server returns `404`.
* [ ] Correct error page is displayed.
* [ ] Server remains operational.

## 4.4 Directory listing

Request a directory.

Verify:

* [ ] Directory listing is displayed when enabled.
* [ ] Directory listing is not displayed when disabled.
* [ ] Default file takes precedence when configured.

## 4.5 Redirects

Request a configured redirect.

Verify:

* [ ] Correct redirect status is returned.
* [ ] `Location` header is correct.
* [ ] Browser follows the redirect correctly.

## 4.6 CGI

Test CGI through the browser where applicable.

Verify:

* [ ] CGI executes.
* [ ] CGI output is returned correctly.
* [ ] CGI errors are handled.
* [ ] CGI `PATH_INFO` works.
* [ ] Relative paths work from the expected working directory.

---

# 5. CGI

At least one CGI implementation must be supported.

## 5.1 CGI execution

* [ ] Correct CGI is selected based on file extension.
* [ ] CGI executes in a child process.
* [ ] Request body is provided through stdin.
* [ ] EOF is correctly provided after the request body.
* [ ] Required CGI environment variables are set.
* [ ] `PATH_INFO` is correctly populated.
* [ ] CGI executes from the correct working directory.
* [ ] CGI stdout is captured correctly.
* [ ] CGI output is converted into a valid HTTP response.

## 5.2 Chunked CGI

* [ ] CGI works when request body uses chunked encoding.
* [ ] Chunks are decoded correctly before CGI receives the body.
* [ ] CGI receives the correct body.
* [ ] File/body contents are not corrupted.

## 5.3 Unchunked CGI

* [ ] CGI works with `Content-Length`.
* [ ] CGI receives exactly the expected body.
* [ ] EOF is correctly provided.

## 5.4 CGI failures

Test:

* missing CGI executable;
* CGI exits with an error;
* malformed CGI output;
* CGI closes unexpectedly;
* CGI takes too long.

Verify:

* [ ] Server does not crash.
* [ ] Client receives an appropriate error.
* [ ] CGI child is cleaned up.
* [ ] Other clients continue to work.

---

# 6. Port Issues

## 6.1 Multiple ports

* [ ] Multiple ports can be configured.
* [ ] All valid ports can listen simultaneously.
* [ ] Requests to each port reach the correct configuration.
* [ ] Different websites/configurations can coexist.

## 6.2 Duplicate ports

Configure the same port multiple times.

Verify:

* [ ] Configuration conflict is detected.
* [ ] The server reports the problem clearly.
* [ ] The server does not silently behave incorrectly.
* [ ] No resource leak occurs after the configuration failure.

## 6.3 Shared ports / independent configurations

Configure multiple servers simultaneously with different configurations but common ports.

Verify:

* [ ] Valid configurations continue to function where the project requirements allow it.
* [ ] An invalid configuration does not unnecessarily bring down unrelated valid configurations.
* [ ] Configuration validation is isolated appropriately.
* [ ] The behavior is deterministic and explainable.

The student must be able to explain how configuration conflicts are detected and handled.

---

# 7. HTTP/1.1

## 7.1 Request parsing

* [ ] HTTP/1.1 request line is parsed.
* [ ] HTTP method is parsed.
* [ ] Request target is parsed.
* [ ] HTTP version is validated.
* [ ] Headers are parsed.
* [ ] Header names are treated case-insensitively where required.

## 7.2 Incremental requests

The server must not assume one read contains one complete request.

Test:

* [ ] Request line split across reads.
* [ ] Headers split across reads.
* [ ] Body split across reads.
* [ ] Chunked data split across reads.

## 7.3 Persistent connections

* [ ] HTTP/1.1 keep-alive works.
* [ ] Multiple requests can be sent on one connection where appropriate.
* [ ] Responses are correctly delimited.
* [ ] Connection close behavior is correct.
* [ ] Idle connections eventually timeout where configured.

## 7.4 Headers

Verify appropriate handling of:

* [ ] `Host`
* [ ] `Content-Length`
* [ ] `Transfer-Encoding`
* [ ] `Connection`
* [ ] `Content-Type`
* [ ] `Cookie`
* [ ] `Set-Cookie`
* [ ] `Location`

---

# 8. Chunked Transfer Encoding

* [ ] Chunked request bodies are supported.
* [ ] Chunk size is parsed correctly.
* [ ] Chunk data is read correctly.
* [ ] Chunk terminators are validated.
* [ ] Multiple chunks work.
* [ ] Zero-length terminating chunk works.
* [ ] Trailers are handled appropriately.
* [ ] Malformed chunk sizes are rejected.
* [ ] Malformed chunk terminators are rejected.
* [ ] Chunked data can be fragmented across reads.
* [ ] Body-size limits apply to decoded request bodies.
* [ ] Server does not crash on malformed chunked input.

---

# 9. Error Handling

The server must provide default error pages for at least:

* [ ] `400 Bad Request`
* [ ] `403 Forbidden`
* [ ] `404 Not Found`
* [ ] `405 Method Not Allowed`
* [ ] `413 Payload Too Large`
* [ ] `500 Internal Server Error`

Verify:

* [ ] Correct status code is returned.
* [ ] Appropriate error body is returned.
* [ ] Custom error pages work when configured.
* [ ] Missing custom error pages fall back safely.
* [ ] Errors do not crash the server.
* [ ] Errors do not expose unnecessary internal information.

---

# 10. Filesystem Security

## 10.1 Path traversal

Test:

```text
../
../../
../../../
```

and encoded/equivalent variants.

Verify:

* [ ] Requests cannot escape configured route roots.
* [ ] Encoded traversal is handled safely.
* [ ] Absolute filesystem paths cannot bypass routing.
* [ ] Symlink behavior is understood and handled appropriately for the project's requirements.

## 10.2 Permissions

* [ ] Forbidden files/directories return `403` where appropriate.
* [ ] Missing resources return `404`.
* [ ] Filesystem errors do not crash the server.

---

# 11. Timeouts and Hanging Connections

The server must not leave clients hanging indefinitely.

Test:

* [ ] Client connects but sends no request.
* [ ] Client sends incomplete headers.
* [ ] Client sends an incomplete body.
* [ ] Client sends data very slowly.
* [ ] Client stops consuming response data.
* [ ] Persistent connection remains idle.
* [ ] CGI takes too long where applicable.

Verify:

* [ ] Timeout occurs.
* [ ] Client is removed from epoll.
* [ ] Socket is closed.
* [ ] Connection state is released.
* [ ] Other clients continue to work.

---

# 12. Memory and Resource Management

## 12.1 Memory

* [ ] No obvious memory leak during normal requests.
* [ ] No unbounded allocation based on client-controlled input.
* [ ] Connection buffers are released after disconnect.
* [ ] Request buffers are released.
* [ ] Response buffers are released.
* [ ] CGI buffers are released.

Useful tools may include:

```text
top
htop
valgrind
AddressSanitizer
```

depending on the environment.

## 12.2 File descriptors

Check that file descriptor count does not continually increase.

Verify cleanup of:

* [ ] client sockets
* [ ] listening sockets on shutdown
* [ ] files
* [ ] epoll descriptors
* [ ] CGI pipes
* [ ] CGI processes

## 12.3 CGI processes

* [ ] CGI children are correctly reaped.
* [ ] Zombie processes do not accumulate.
* [ ] CGI failure does not leave resources behind.

---

# 13. Stress Testing

Use:

```bash
siege -b [IP]:[PORT]
```

The server must achieve:

```text
availability >= 99.5%
```

During the test verify:

* [ ] Server remains alive.
* [ ] Server does not crash.
* [ ] Requests continue receiving valid responses.
* [ ] Availability is at least 99.5%.
* [ ] No hanging connections occur.
* [ ] Memory does not continuously increase.
* [ ] File descriptor count does not continuously increase.
* [ ] CPU behavior is reasonable.
* [ ] Connections are correctly cleaned up.

Repeat the test after major changes.

---

# 14. Concurrency

Test multiple simultaneous clients.

Verify:

* [ ] One slow client does not block other clients.
* [ ] One large request does not block other clients.
* [ ] One slow response does not block other clients.
* [ ] Multiple clients can download files simultaneously.
* [ ] Multiple persistent connections work.
* [ ] Client failures do not affect unrelated clients.

---

# 15. Source Code Audit

The auditor should be able to quickly locate the implementation of each major requirement.

## Event loop

* [ ] Location of epoll initialization is obvious.
* [ ] Location of `epoll_ctl`/registration is obvious.
* [ ] Location of `epoll_wait` is obvious.
* [ ] Main event loop is easy to identify.

## Client reading

* [ ] Client read path is obvious.
* [ ] Read return values are checked.
* [ ] EAGAIN/EWOULDBLOCK is handled.
* [ ] Client disconnect is handled.
* [ ] Fatal errors remove the client.
* [ ] Only one read occurs per client per event.

## Client writing

* [ ] Client write path is obvious.
* [ ] Write return values are checked.
* [ ] Partial writes are handled.
* [ ] EAGAIN/EWOULDBLOCK is handled.
* [ ] Only one write occurs per client per event.

## HTTP

* [ ] Request parser is easy to locate.
* [ ] Response generation is easy to locate.
* [ ] Chunked parsing is easy to locate.
* [ ] HTTP status generation is easy to locate.

## Routing

* [ ] Server selection is easy to locate.
* [ ] Hostname matching is easy to locate.
* [ ] Route matching is easy to locate.
* [ ] Filesystem mapping is easy to locate.

## CGI

* [ ] CGI detection is easy to locate.
* [ ] CGI process creation is easy to locate.
* [ ] Environment setup is easy to locate.
* [ ] PATH_INFO setup is easy to locate.
* [ ] CGI cleanup is easy to locate.

## Configuration

* [ ] Configuration parser is easy to locate.
* [ ] Configuration validation is easy to locate.
* [ ] Port conflict handling is easy to locate.

---

# 16. Student Explanation / Mock Interview

The student should be able to answer all of the following without relying solely on documentation.

## Networking

* [ ] How does an HTTP server work?
* [ ] What happens when a browser connects to the server?
* [ ] What happens when the TCP connection is established?
* [ ] What does the listening socket do?
* [ ] What does `accept()` do?
* [ ] Why are sockets non-blocking?
* [ ] Why is I/O multiplexing necessary?
* [ ] Why was epoll/select/poll chosen?
* [ ] How does epoll work?
* [ ] What is the difference between a listening socket and a client socket?

## Event loop

* [ ] Where is the event loop?
* [ ] Why is there only one event loop?
* [ ] Why is there only one multiplexing mechanism?
* [ ] How does a readable event become a `read()`?
* [ ] How does a writable event become a `write()`?
* [ ] Why is there only one read/write per client per event?
* [ ] How are partial reads handled?
* [ ] How are partial writes handled?
* [ ] What happens when `read()` returns `0`?
* [ ] What happens when `read()` returns an error?
* [ ] What happens when `write()` returns an error?
* [ ] What is `EAGAIN`?
* [ ] What is `EWOULDBLOCK`?
* [ ] What is `EINTR`?

## HTTP

* [ ] What is an HTTP request?
* [ ] What is an HTTP response?
* [ ] What is the request line?
* [ ] What are HTTP headers?
* [ ] Why is the `Host` header important?
* [ ] What is `Content-Length`?
* [ ] What is chunked transfer encoding?
* [ ] Why can't the server assume one read contains the entire request?
* [ ] Why can't the server assume one write sends the entire response?
* [ ] What is HTTP keep-alive?

## Routing

* [ ] How is a virtual host selected?
* [ ] How is a route selected?
* [ ] How is a URL converted into a filesystem path?
* [ ] How is path traversal prevented?
* [ ] How are allowed methods enforced?
* [ ] How are redirects implemented?

## CGI

* [ ] Why does CGI require another process?
* [ ] How does the server send the request body to CGI?
* [ ] How does CGI know when the request body has ended?
* [ ] What is `PATH_INFO`?
* [ ] Why does CGI need a working directory?
* [ ] How does the parent avoid blocking the entire server while CGI runs?

## Configuration

* [ ] How is configuration parsed?
* [ ] How are invalid configurations detected?
* [ ] How are multiple ports handled?
* [ ] How are multiple virtual hosts handled?
* [ ] What happens when ports conflict?
* [ ] How is the default server selected?

## Robustness

* [ ] How are timeouts implemented?
* [ ] How does the server prevent a slow client from blocking everyone?
* [ ] How are malformed requests handled?
* [ ] How are resource leaks prevented?
* [ ] How were memory leaks tested?
* [ ] How were hanging connections tested?
* [ ] How was the 99.5% siege requirement verified?

---

# 17. Final Audit Sign-Off

Do not consider the project audit-ready until all applicable items above have been verified.

## Functional

* [ ] HTTP server works.
* [ ] Browser works.
* [ ] GET works.
* [ ] POST works.
* [ ] DELETE works.
* [ ] Uploads work.
* [ ] Cookies work.
* [ ] Sessions work.
* [ ] CGI works.
* [ ] Chunked requests work.
* [ ] Unchunked requests work.
* [ ] Routing works.
* [ ] Redirects work.
* [ ] Directory listing works.
* [ ] Default files work.
* [ ] Error pages work.
* [ ] Body limits work.

## Networking

* [ ] One process.
* [ ] One server thread.
* [ ] One central event loop.
* [ ] Non-blocking sockets.
* [ ] Event-driven reads.
* [ ] Event-driven writes.
* [ ] One read/write per client per event.
* [ ] Socket errors handled.
* [ ] Client cleanup works.
* [ ] Timeouts work.

## Configuration

* [ ] Multiple ports.
* [ ] Multiple servers.
* [ ] Virtual hosts.
* [ ] Custom error pages.
* [ ] Configuration validation.
* [ ] Port conflict handling.

## Reliability

* [ ] No crashes under malformed input.
* [ ] No hanging connections.
* [ ] No obvious memory leaks.
* [ ] No file descriptor leaks.
* [ ] CGI children are cleaned up.
* [ ] Siege availability >= 99.5%.

## Audit readiness

* [ ] Student can explain the architecture.
* [ ] Student can explain the event loop.
* [ ] Student can explain the one-read/one-write rule.
* [ ] Student can explain HTTP parsing.
* [ ] Student can explain routing.
* [ ] Student can explain CGI.
* [ ] Student can explain configuration.
* [ ] Student can explain error handling.
* [ ] Student can explain timeout handling.
* [ ] Student can explain resource cleanup.
* [ ] Student can demonstrate the relevant source code for each requirement.
