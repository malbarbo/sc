/* The engine's API (PLANO-REPL.md §2). Only scalars and pointers, so the
   signatures are the same on wasm32 and x86-64. The Rust side takes the
   types and declarations from here through bindgen (proto/).

   A result belongs to the engine and is valid until the next call on the
   same session. The engine is not thread safe: one thread uses it. A fatal
   error in the compiler ends the process, so the native REPL runs a session
   in a child process. */
#ifndef SC_H
#define SC_H

#ifdef __cplusplus
extern "C" {
#endif

enum sc_status {
  SC_OK = 0,    /* done */
  SC_ERROR = 1, /* an error in the student's code. A failed cell leaves part of
                   itself in the session (§3.3); the child of the protocol
                   goes on from a copy taken before it */
  SC_LOST = 2,  /* after a fatal error, a failed link or an undo: the session must be
                   recreated and the log replayed (§3.6) */
};

enum sc_part {
  SC_DIAGS = 0, /* the diagnostics of the call, as text */
  SC_SO = 1,    /* the cell as a wasm shared object */
  SC_SYMS = 2,  /* the names the cell exports, one per line, to check a replay */
};

enum sc_cell_mode {
  SC_CELL_SO = 0,     /* compile and link the cell: SC_SO and SC_SYMS */
  SC_CELL_REPLAY = 1, /* parse and generate IR only, as in a replay: SC_SYMS */
};

typedef struct sc_session sc_session;

/* A session. argv are cc1 flags without an input file, -x c or -x c++ (C by
   default), then, after "--", the wasm-ld arguments that link a cell (-L,
   the libraries, the builtins). *out is always set, also when the session
   could not start (SC_ERROR, with SC_DIAGS): free it with sc_session_free. */
int sc_session_new(int argc, const char *const *argv, sc_session **out);
void sc_session_free(sc_session *s);
/* Compiles src[0, len) on top of the cells before (mode: enum sc_cell_mode). */
int sc_session_cell(sc_session *s, const char *src, unsigned len, int mode);
/* Undoes the last accepted cell. Its code is already generated, so this is
   SC_LOST: the host replays the log without it (§3.3). SC_ERROR if no cell
   was accepted. */
int sc_session_undo(sc_session *s);
/* A part of the result of the last call (enum sc_part). */
const unsigned char *sc_result_ptr(sc_session *s, int part);
unsigned sc_result_len(sc_session *s, int part);

/* The one-shot engine: sc -cc1 ..., sc ld ... (argv[0] is the program). */
int sc_main(int argc, const char **argv);

/* The protocol of the native child (sc-engine session, proto/): each message
   is the header, in the host's byte order, then len[0], len[1] and len[2]
   bytes. The child answers each request with one SC_MSG_REPLY: its status,
   and the parts in the order of enum sc_part. It ends at the end of its
   input.
     SC_MSG_NEW   a session (it replaces the one before): the arguments of
                  sc_session_new, each followed by a NUL, in the first part
     SC_MSG_CELL  sc_session_cell: the source in the first part, the mode in
                  status. In SC_CELL_SO the child forks a copy of itself
                  first; if the cell fails or ends the process, the copy
                  answers SC_ERROR and goes on, so the session is as before
                  the cell
     SC_MSG_UNDO  sc_session_undo */
enum sc_msg { SC_MSG_NEW = 1, SC_MSG_CELL = 2, SC_MSG_UNDO = 3, SC_MSG_REPLY = 4 };
struct sc_msg_header {
  unsigned kind, status, len[3];
};

#ifdef __cplusplus
}
#endif

#endif
