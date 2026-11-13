#!/usr/bin/env python3
"""Talks to `claw lsp` the way an editor does and checks what comes back."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAW = pathlib.Path(os.environ.get("CLAW", ROOT / "compiler/target/release/claw"))


class Client:
    def __init__(self):
        self.p = subprocess.Popen([CLAW, "lsp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.next_id = 0
        self.pending = []

    def send(self, obj):
        body = json.dumps(obj).encode()
        self.p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        self.p.stdin.flush()

    def read(self):
        length = 0
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise EOFError("server closed")
            line = line.strip()
            if not line:
                break
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":")[1])
        return json.loads(self.p.stdout.read(length))

    def request(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        while True:
            m = self.read()
            if m.get("id") == self.next_id:
                return m
            self.pending.append(m)

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def diagnostics(self, uri):
        """The last publishDiagnostics for `uri`, waiting if it has not come yet."""
        found = None
        for m in self.pending:
            if m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri:
                found = m["params"]["diagnostics"]
        self.pending = []
        while found is None:
            m = self.read()
            if m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri:
                found = m["params"]["diagnostics"]
        return found


def check(cond, what, failures):
    if not cond:
        failures.append(what)


def main():
    failures = []
    with tempfile.TemporaryDirectory() as d:
        path = pathlib.Path(d).resolve() / "demo file.claw"
        uri = "file://" + str(path).replace(" ", "%20")
        good = "pub fn add(a int, b int) int {\n    ret a + b\n}\n\nfn main() {\n    print(add(1, 2))\n}\n"
        bad = "fn main() {\n    x := 5\n    print(1)\n}\n"
        path.write_text(good)
        c = Client()
        r = c.request("initialize", {"processId": None, "rootUri": None, "capabilities": {}})
        caps = r["result"]["capabilities"]
        check(caps.get("hoverProvider") and caps.get("definitionProvider"), "capabilities", failures)
        c.notify("initialized", {})

        # An unsaved buffer with an error: the editor's text is what gets checked, not the disk.
        c.notify("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "claw", "version": 1, "text": bad}})
        diags = c.diagnostics(uri)
        check(len(diags) == 1 and diags[0]["code"] == "E0105", f"unused variable reported: {diags}", failures)
        if diags:
            check(diags[0]["range"]["start"] == {"line": 1, "character": 4}, f"position of the error: {diags[0]['range']}", failures)
            check("fix:" in diags[0]["message"], "message carries the fix", failures)
        acts = c.request("textDocument/codeAction", {"textDocument": {"uri": uri}, "range": diags[0]["range"], "context": {"diagnostics": diags}})["result"]
        check(len(acts) == 1 and acts[0]["edit"]["changes"][uri][0]["newText"] == "_ = ", f"quick fix: {acts}", failures)

        # The error goes away when the text is fixed, and hover and definition work on the checked program.
        c.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": good}]})
        check(c.diagnostics(uri) == [], "errors cleared after the fix", failures)
        hover = c.request("textDocument/hover", {"textDocument": {"uri": uri}, "position": {"line": 5, "character": 12}})["result"]
        check(hover and "pub fn add(a int, b int) int" in hover["contents"]["value"], f"hover on a call: {hover}", failures)
        where = c.request("textDocument/definition", {"textDocument": {"uri": uri}, "position": {"line": 5, "character": 12}})["result"]
        check(where and where["uri"] == uri and where["range"]["start"]["line"] == 0, f"definition: {where}", failures)
        syms = c.request("textDocument/documentSymbol", {"textDocument": {"uri": uri}})["result"]
        check([s["name"] for s in syms] == ["add", "main"], f"outline: {syms}", failures)

        # Formatting is `claw fmt`.
        c.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": "fn   main( ) {\nprint( 1 )\n}\n"}]})
        c.diagnostics(uri)
        edits = c.request("textDocument/formatting", {"textDocument": {"uri": uri}, "options": {"tabSize": 4, "insertSpaces": True}})["result"]
        check(len(edits) == 1 and edits[0]["newText"] == "fn main() {\n    print(1)\n}\n", f"formatting: {edits}", failures)

        # A request it does not know is an error response, not a crash.
        r = c.request("workspace/nonsense", {})
        check("error" in r, "unknown method answered with an error", failures)
        c.request("shutdown", None)
        c.notify("exit", None)
        check(c.p.wait(timeout=10) == 0, "clean exit after shutdown", failures)
    if failures:
        print("\n".join("FAIL " + f for f in failures))
        sys.exit(1)
    print("lsp ok")


if __name__ == "__main__":
    main()
