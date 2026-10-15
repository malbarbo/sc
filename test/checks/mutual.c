int par(int n);

int impar(int n) { return n == 0 ? 0 : par(n - 1); }

int par(int n) { return n == 0 ? 1 : impar(n + 1); }

int main(void) { return par(4); }
