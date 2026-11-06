// error handling with setjmp and longjmp: an expression calculator
#include <ctype.h>
#include <setjmp.h>
#include <stdio.h>

static jmp_buf failure;
static const char *msg;
static const char *p;

static void fail(const char *m) {
    msg = m;
    longjmp(failure, 1);
}

static int expr(void);

static int factor(void) {
    while (*p == ' ') p++;
    if (*p == '(') {
        p++;
        int v = expr();
        if (*p != ')') fail("missing )");
        p++;
        return v;
    }
    if (!isdigit((unsigned char)*p)) fail("number expected");
    int v = 0;
    while (isdigit((unsigned char)*p)) v = v * 10 + (*p++ - '0');
    return v;
}

static int term(void) {
    int v = factor();
    for (;;) {
        while (*p == ' ') p++;
        if (*p == '*') {
            p++;
            v *= factor();
        } else if (*p == '/') {
            p++;
            int d = factor();
            if (d == 0) fail("division by zero");
            v /= d;
        } else {
            return v;
        }
    }
}

static int expr(void) {
    int v = term();
    for (;;) {
        while (*p == ' ') p++;
        if (*p == '+') {
            p++;
            v += term();
        } else if (*p == '-') {
            p++;
            v -= term();
        } else {
            return v;
        }
    }
}

int main(void) {
    const char *tests[] = {"1 + 2 * 3", "(1 + 2) * 3", "8 / (4 - 4)", "2 * (3 + 4", "7 - x"};
    for (int i = 0; i < 5; i++) {
        p = tests[i];
        if (setjmp(failure) == 0) {
            int v = expr();
            printf("%s = %d\n", tests[i], v);
        } else {
            printf("%s: error: %s\n", tests[i], msg);
        }
    }
    return 0;
}
