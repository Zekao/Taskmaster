/*
    This file is used to test the failure of the program.

    In the configuration file, we can specify which return codes are considered
    as a success. If we don't encounter any of these return codes, the program
    will be considered as a failure and should be restarted if we have the
   restart option.
*/

int main(void) { return 42; }
