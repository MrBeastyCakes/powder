// Hands control to the Rust app; it never returns.
extern void powder_toy_main(void);

int main(int argc, char *argv[]) {
    powder_toy_main();
    return 0;
}
