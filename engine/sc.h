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
  SC_INCOMPLETE = 3, /* SC_CELL_WHOLE: the cell stops short, as a failed cell
                        (SC_ERROR) without its diagnostics */
};

enum sc_part {
  SC_DIAGS = 0, /* the diagnostics of the call, as text */
  SC_SO = 1,    /* the cell as a wasm shared object */
  SC_SYMS = 2,  /* the names the cell exports, one per line, to check a
                   replay; SC_CELL_TYPE: the type */
};

enum sc_cell_mode {
  SC_CELL_SO = 0,     /* compile and link the cell: SC_SO and SC_SYMS */
  SC_CELL_REPLAY = 1, /* parse and generate IR only, as in a replay: SC_SYMS */
  SC_CELL_WHOLE = 2,  /* SC_CELL_SO, or SC_INCOMPLETE if the cell stops short:
                         its first error is at its end, where the parser wanted
                         more (a declaration without its `;`, a function's
                         header without its body, an `if` without its
                         statement). The REPL asks so on Enter */
  SC_CELL_TYPE = 3,   /* the cell is an expression without its `;`: SC_OK with
                         its type in SC_SYMS, as a shown value writes it
                         (below). Nothing is generated, but as a failed cell
                         (SC_ERROR) it leaves part of itself in the session */
};

/* A cell that ends in an expression without its `;` shows its value, if the
   value is a number, a character, a bool or a pointer: the expression
   becomes a call to
     void __sc_show_int(const char *type, int kind, long long value);
     void __sc_show_float(const char *type, int kind, double value);
   which the host that loads the cells defines. type is the expression's type
   as clang writes it: a typedef at namespace scope (size_t, std::string)
   keeps its name, the rest is canonical; kind is one of these. A string is the address of its
   first char. Any other value is not shown. */
enum sc_show {
  SC_SHOW_SIGNED = 0,
  SC_SHOW_UNSIGNED = 1,
  SC_SHOW_BOOL = 2,
  SC_SHOW_CHAR = 3,
  SC_SHOW_POINTER = 4,
  SC_SHOW_STRING = 5,  /* a char * or a char array */
  SC_SHOW_NULLPTR = 6, /* nullptr_t */
  SC_SHOW_DOUBLE = 7,  /* __sc_show_float: double, long double */
  SC_SHOW_FLOAT = 8,   /* __sc_show_float: float */
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
/* sc.wasm with SC_SNAPSHOT (sc_api.cpp): after a trap or an exit in
   sc_session_cell, the session as it was before the cell, from the copy the
   cell took, and SC_ERROR; SC_LOST if there is none (the host replays the
   log). The host calls it first, before any other export. */
int sc_session_recover(sc_session *s);
/* The brackets src[0, len) leaves open, by the session's lexer, one more if
   it ends inside a comment or with a `\` that joins the next line: how far
   in the REPL starts the line Enter opens. It does not touch the session. */
int sc_session_depth(sc_session *s, const char *src, unsigned len);
/* A part of the result of the last call (enum sc_part). */
const unsigned char *sc_result_ptr(sc_session *s, int part);
unsigned sc_result_len(sc_session *s, int part);

/* The files a session reads in place of the disk: those under root (the
   pack the native sc embeds, PLANO-REPL.md §5.4), for its clang and its
   wasm-ld. The rest are on the disk, and so is all without them and in the
   one-shot engine. sc.wasm does not export it: it has its own pack, which
   its host turns on with sc_pack (sc_pack.cpp). A path
   asked for is absolute and without "." or "..", with '/' or the system's
   separator. What read and list return is valid until the process ends. */
struct sc_files {
  const char *root; /* "/sc" */
  /* 0: no such path; 1: a file of *size bytes; 2: a directory */
  int (*status)(const char *path, unsigned long long *size);
  /* a file's *size bytes, with a NUL after them; NULL if it is not a file */
  const char *(*read)(const char *path, unsigned long long *size);
  /* the names in a directory, each followed by a NUL, *size bytes in all;
     NULL if it is not a directory */
  const char *(*list)(const char *path, unsigned long long *size);
};
void sc_set_files(const struct sc_files *files);

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
                  status. In SC_CELL_SO, SC_CELL_WHOLE and SC_CELL_TYPE the
                  child forks a copy of itself first; if the cell fails or
                  ends the process, the copy answers SC_ERROR (or
                  SC_INCOMPLETE) and goes on, so the session is as before
                  the cell. After SC_CELL_TYPE the copy always goes on
     SC_MSG_UNDO  sc_session_undo
     SC_MSG_DEPTH sc_session_depth: the source in the first part; the reply's
                  status is the depth */
enum sc_msg { SC_MSG_NEW = 1, SC_MSG_CELL = 2, SC_MSG_UNDO = 3, SC_MSG_REPLY = 4, SC_MSG_DEPTH = 5 };
struct sc_msg_header {
  unsigned kind, status, len[3];
};

#ifdef __cplusplus
}
#endif

#endif
