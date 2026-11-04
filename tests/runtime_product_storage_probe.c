/* Small host-built CI payload; no replacement image/runtime is involved. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 3) return 64;
    char data[65536]; memset(data, 's', sizeof(data));
    if (!strcmp(argv[1], "fill")) {
        int fd = open(argv[2], O_CREAT|O_WRONLY|O_TRUNC, 0600);
        if (fd < 0) { perror("open"); return 1; }
        size_t count = 0;
        for (;;) {
            ssize_t n = write(fd, data, sizeof(data));
            if (n < 0) break;
            count += (size_t)n;
        }
        int error = errno;
        printf("{\"path\":\"%s\",\"written_bytes\":%zu,\"write_errno\":%d,", argv[2], count, error);
        errno = 0; int synced = fsync(fd), sync_error = errno;
        printf("\"fsync_result\":%d,\"fsync_errno\":%d}\n", synced, sync_error);
        close(fd);
        return error == ENOSPC || error == EDQUOT ? 0 : 1;
    }
    if (!strcmp(argv[1], "pwrite")) {
        int fd = open(argv[2], O_CREAT|O_WRONLY, 0600);
        if (fd < 0) { int error = errno; printf("{\"open_errno\":%d}\n", error); return error == ENOSPC || error == EDQUOT ? 0 : 1; }
        ssize_t n;
        off_t offset = lseek(fd, 0, SEEK_END);
        do { errno = 0; n = pwrite(fd, data, sizeof(data), offset); if (n > 0) offset += n; } while (n >= 0);
        int error = errno;
        printf("{\"pwrite_result\":%zd,\"pwrite_errno\":%d}\n", n, error);
        close(fd);
        return n < 0 && (error == ENOSPC || error == EDQUOT) ? 0 : 1;
    }
    return 64;
}
