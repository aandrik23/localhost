# Localhost — Agent Instructions

## Project

This repository implements the Localhost HTTP/1.1 server project.

The authoritative project requirements are:

* `docs/project-spec.md`
* `docs/audit-checklist.md`

Read both before making architectural decisions.

## Core constraints

The server must:

* be written in Rust;
* use one server process;
* use one server thread;
* support multiple listening ports and virtual servers;
* use epoll or an equivalent I/O multiplexing mechanism;
* use non-blocking network I/O;
* implement the HTTP server rather than relying on a server framework.

Do not use:

* Tokio
* async-std
* smol
* mio
* nix
* actix
* hyper as the server implementation
* equivalent networking/event-loop frameworks

`libc` is allowed for required system calls.

## Critical audit requirement

The event loop must have one central multiplexing mechanism.

For each client returned by the multiplexing operation:

* perform at most one read operation OR one write operation;
* do not loop on read until EAGAIN;
* do not loop on write until the response is complete;
* return to the event loop and allow the multiplexer to signal the socket again.

Partial reads and writes must be represented through connection state.

The implementation must make this behavior obvious and easy to explain during the audit.

## Engineering requirements

* Prefer simple, explicit, auditable code.
* Avoid unnecessary abstractions.
* Avoid `unwrap()`/`expect()` on runtime/network/client-controlled operations.
* Check system call return values.
* Handle EAGAIN/EWOULDBLOCK/EINTR correctly.
* Never allow one client failure to crash the server.
* Clean up sockets, file descriptors, buffers, and child processes.
* Never introduce blocking I/O into the server event loop.
* Do not silently weaken requirements.

## Development process

Do not implement the entire project in one step.

Work in explicit phases.

Before making major architectural changes:

1. inspect the existing code;
2. explain the proposed approach;
3. identify relevant requirements;
4. implement the smallest coherent phase;
5. compile;
6. test;
7. inspect failures;
8. fix them;
9. only then proceed.

Do not rewrite working code without a concrete reason.

## Auditability

The following should each have an obvious location in the source code:

* epoll initialization;
* epoll registration;
* main event loop;
* client read;
* client write;
* client cleanup;
* HTTP parsing;
* routing;
* response generation;
* configuration parsing;
* CGI execution.

The student must be able to explain the implementation during an audit by following the source code.

## Important

Do not claim a requirement is satisfied without testing or inspecting the implementation.

When a requirement is ambiguous, stop and explain the ambiguity before choosing a design that could affect the audit.
