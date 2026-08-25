#!/usr/bin/env python3
import os
import sys
import html

body = sys.stdin.read()

method = os.environ.get("REQUEST_METHOD", "")
path_info = os.environ.get("PATH_INFO", "")
query = os.environ.get("QUERY_STRING", "")
cookie = os.environ.get("HTTP_COOKIE", "(none)")
remote_addr = os.environ.get("REMOTE_ADDR", "")

posted_message = ""
if method == "POST" and body:
    for pair in body.split("&"):
        if pair.startswith("message="):
            posted_message = pair.split("=", 1)[1].replace("+", " ")

print("Content-Type: text/html")
print()
print("<!DOCTYPE html>")
print("<html lang=\"en\"><head><meta charset=\"UTF-8\">")
print("<title>CGI Test</title>")
print("<link rel=\"icon\" href=\"/favicon.ico\">")
print("<link rel=\"stylesheet\" href=\"/css/style.css\"></head><body>")
print("<header><img src=\"/images/logo.png\" alt=\"logo\">"
      "<h1>CGI Test (Python)</h1></header>")
print("<nav><a href=\"/\">Home</a></nav>")

print("<ul>")
print(f"<li>REQUEST_METHOD={html.escape(method)}</li>")
print(f"<li>PATH_INFO={html.escape(path_info)}</li>")
print(f"<li>QUERY_STRING={html.escape(query)}</li>")
print(f"<li>HTTP_COOKIE={html.escape(cookie)}</li>")
print(f"<li>REMOTE_ADDR={html.escape(remote_addr)}</li>")
print("</ul>")

if posted_message:
    print(f"<p>You posted: <strong>{html.escape(posted_message)}"
          "</strong></p>")

print("<form method=\"POST\" action=\"/cgi/hello.py\">")
print("<label for=\"message\">Message:</label><br>")
print("<input type=\"text\" id=\"message\" name=\"message\"><br><br>")
print("<button type=\"submit\">Send (POST to CGI)</button>")
print("</form>")

print("</body></html>")
