#!/usr/bin/env python3
import os
import sys

body = sys.stdin.read()

print("Content-Type: text/plain")
print()
print("Hello from CGI")
print("REQUEST_METHOD=" + os.environ.get("REQUEST_METHOD", ""))
print("PATH_INFO=" + os.environ.get("PATH_INFO", ""))
print("QUERY_STRING=" + os.environ.get("QUERY_STRING", ""))
print("CONTENT_LENGTH=" + os.environ.get("CONTENT_LENGTH", ""))
print("BODY=" + body)
