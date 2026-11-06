#!/usr/bin/env python3
"""Ctrl-C in sc's REPL, in a terminal (a pty, TERM=xterm: with TERM=dumb
rustyline leaves raw mode out), NOTES.md §70: during the compile Enter
does, twice in a row, with Ctrl-\\ and Ctrl-Z before it (which do nothing),
at the prompt, and in the student's code. After each, the session goes on.

usage: ctrlc.py SC | ctrlc.py ssh:HOST

ssh:HOST runs the sc.exe of HOST's home, in its console (Windows' OpenSSH
gives it a ConPTY, which makes ^C a Ctrl-C).
"""
import os, pty, re, select, signal, sys, time

SC = sys.argv[1]
# the prompt, coloured or not (NO_COLOR does not get over ssh), and with
# the space as a move of the cursor (ConPTY)
P = rb'>(?:\x1b\[m)?(?: |(?:\x1b\[K)?\x1b\[1C)'
THREE = rb'\(int\)(?:\x1b\[[0-9;]*m)* (?:\x1b\[[0-9;]*m)*3'


def spawn():
    pid, fd = pty.fork()
    if pid == 0:
        os.environ['TERM'] = 'xterm'
        os.environ['NO_COLOR'] = '1'
        if SC.startswith('ssh:'):
            os.execvp('ssh', ['ssh', '-tt', SC[4:], r'.\sc.exe -x=c++ -q'])
        os.execv(SC, [SC, '-x=c++', '-q'])
    return pid, fd


def read_until(fd, pat, timeout):
    buf = b''
    end = time.time() + timeout
    while time.time() < end:
        if select.select([fd], [], [], 0.1)[0]:
            try:
                d = os.read(fd, 4096)
            except OSError:
                break
            if not d:
                break
            buf += d
            if re.search(pat, buf):
                return buf, True
    return buf, False


def alive(pid):
    try:
        return os.waitpid(pid, os.WNOHANG)[0] == 0
    except ChildProcessError:  # reaped
        return False


def case(name, steps, wait):
    """steps: (seconds, bytes) to write, then `wait` in the output; then
    1+2 must give 3."""
    pid, fd = spawn()
    out, ok = read_until(fd, P, 60)
    for delay, k in steps:
        time.sleep(delay)
        os.write(fd, k)
    if ok:
        o, ok = read_until(fd, wait, 30)
        out += o
    if ok and alive(pid):
        # what comes after a Ctrl-C that rustyline reads goes with it
        out += read_until(fd, rb'$^', 1.5)[0]
        os.write(fd, b'1+2\r')
        o, ok = read_until(fd, THREE, 30)
        out += o
    ok = ok and alive(pid)
    print('ok  ' if ok else 'FAIL', name, flush=True)
    if not ok:
        print('   ', out[-400:])
    # Ctrl-D, so that sc ends as it does (its TMPDIR goes)
    os.write(fd, b'\x04')
    end = time.time() + 10
    while alive(pid) and time.time() < end:
        read_until(fd, rb'$^', 0.2)
    if alive(pid):
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    return ok


INTERRUPTED = rb'interrupted[\s\S]*' + P
CELL = b'#include <iostream>\r'
r = [
    case('ctrl-c on enter', [(0, CELL), (0.5, b'\x03')], INTERRUPTED),
    case('two ctrl-c on enter', [(0, CELL), (0.5, b'\x03'), (0.05, b'\x03')], INTERRUPTED),
    case('ctrl-\\ and ctrl-z do nothing', [(0, CELL), (0.3, b'\x1c'), (0.1, b'\x1a'), (0.1, b'\x03')], INTERRUPTED),
    case('ctrl-c at the prompt', [(0, b'int x = '), (0.3, b'\x03')], P),
    case('two ctrl-c in the code', [(0, b'void f() { while (1); }\r'), (1, b'f();\r'), (2, b'\x03'), (0.05, b'\x03')],
         INTERRUPTED),
]
sys.exit(0 if all(r) else 1)
