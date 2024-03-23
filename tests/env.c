#include <stdio.h>

/*
    This file is used to test the environment variables.
*/

int main(int argc, char **argv, char **env) {
  (void)argc;
  (void)argv;

  for (int i = 0; env[i] != NULL; i++) {
    printf("%s\n", env[i]);
  }
}
