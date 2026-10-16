// tratamento de erros com setjmp e longjmp: uma calculadora de expressões
#include <ctype.h>
#include <setjmp.h>
#include <stdio.h>

static jmp_buf falha;
static const char *msg;
static const char *p;

static void erro(const char *m) {
    msg = m;
    longjmp(falha, 1);
}

static int expr(void);

static int fator(void) {
    while (*p == ' ') p++;
    if (*p == '(') {
        p++;
        int v = expr();
        if (*p != ')') erro("falta )");
        p++;
        return v;
    }
    if (!isdigit((unsigned char)*p)) erro("número esperado");
    int v = 0;
    while (isdigit((unsigned char)*p)) v = v * 10 + (*p++ - '0');
    return v;
}

static int termo(void) {
    int v = fator();
    for (;;) {
        while (*p == ' ') p++;
        if (*p == '*') {
            p++;
            v *= fator();
        } else if (*p == '/') {
            p++;
            int d = fator();
            if (d == 0) erro("divisão por zero");
            v /= d;
        } else {
            return v;
        }
    }
}

static int expr(void) {
    int v = termo();
    for (;;) {
        while (*p == ' ') p++;
        if (*p == '+') {
            p++;
            v += termo();
        } else if (*p == '-') {
            p++;
            v -= termo();
        } else {
            return v;
        }
    }
}

int main(void) {
    const char *testes[] = {"1 + 2 * 3", "(1 + 2) * 3", "8 / (4 - 4)", "2 * (3 + 4", "7 - x"};
    for (int i = 0; i < 5; i++) {
        p = testes[i];
        if (setjmp(falha) == 0) {
            int v = expr();
            printf("%s = %d\n", testes[i], v);
        } else {
            printf("%s: erro: %s\n", testes[i], msg);
        }
    }
    return 0;
}
