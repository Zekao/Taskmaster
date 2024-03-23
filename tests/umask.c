#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

/*
    This program is used to testk umask option:
    If the open permissions are 0777 and the umask is 0070
    the file will be created with permissions 0777 & ~0070 = 0707.
*/

int main(void) {
  int fd = open("test.txt", O_CREAT | O_RDWR, 0777);
  if (fd < 0)
    return 1;
  return close(fd), 0;
}
