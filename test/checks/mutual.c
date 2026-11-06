int even(int n);

int odd(int n) { return n == 0 ? 0 : even(n - 1); }

int even(int n) { return n == 0 ? 1 : odd(n + 1); }

int main(void) { return even(4); }
