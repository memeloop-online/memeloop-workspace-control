/* Small host-built CI payload; no replacement image/runtime is involved. */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 3) return 64;
    char data[65536]; memset(data, 's', sizeof(data));
    if (!strcmp(argv[1], "write") || !strcmp(argv[1], "emit")) {
        if (argc < 4) return 64;
        char *end = NULL;
        errno = 0;
        unsigned long long bytes = strtoull(argv[3], &end, 10);
        if (errno || !end || *end || !bytes || argv[3][0] == '-') return 64;
        int emit = !strcmp(argv[1], "emit");
        int fd = open(argv[2], emit ? O_WRONLY : O_CREAT|O_WRONLY|O_TRUNC, 0600);
        if (fd < 0) { perror("open"); return 1; }
        if (emit) {
            if (argc != 5 || strlen(argv[4]) > 128) { close(fd); return 64; }
            /* Each marker is a complete short CRI line, never a 64-KiB partial. */
            for (size_t offset = 0; offset < sizeof(data); offset += 1024) {
                memcpy(data+offset, argv[4], strlen(argv[4]));
                data[offset+strlen(argv[4])] = ' ';
                data[offset+1023] = '\n';
            }
        }
        unsigned long long count = 0;
        while (count < bytes) {
            size_t chunk = bytes-count < sizeof(data) ? (size_t)(bytes-count) : sizeof(data);
            ssize_t n = write(fd, data, chunk);
            if (n < 0 && errno == EINTR) continue;
            if (n <= 0) { perror("write"); close(fd); return 1; }
            count += (unsigned long long)n;
        }
        int synced = emit ? 0 : fsync(fd);
        int sync_error = synced < 0 ? errno : 0;
        if (close(fd) < 0) { perror("close"); return 1; }
        printf("{\"written_bytes\":%llu,\"fsync_result\":%d,\"fsync_errno\":%d,\"stdout_emission\":%s}\n", count, synced, sync_error, emit ? "true" : "false");
        return synced == 0 ? 0 : 1;
    }
    if (!strcmp(argv[1], "fill")) {
        int fd = open(argv[2], O_CREAT|O_WRONLY|O_APPEND, 0600);
        if (fd < 0) { perror("open"); return 1; }
        size_t count = 0;
        for (;;) {
            ssize_t n = write(fd, data, sizeof(data));
            if (n < 0 && errno == EINTR) continue;
            if (n <= 0) break;
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
        off_t original_size = lseek(fd, 0, SEEK_END);
        if (original_size < 0) { perror("lseek"); close(fd); return 1; }
        off_t offset = original_size;
        /* Append only comments and restore the original length even on error.
         * Existing managed-file bytes, especially /etc/hosts, are never overwritten. */
        data[0] = '\n'; data[1] = '#'; data[sizeof(data)-1] = '\n';
        do {
            errno = 0;
            n = pwrite(fd, data, sizeof(data), offset);
            if (n < 0 && errno == EINTR) continue;
            if (n <= 0) break;
            offset += n;
        } while (1);
        int error = errno;
        int restored = ftruncate(fd, original_size);
        int synced = fsync(fd);
        int closed = close(fd);
        printf("{\"pwrite_result\":%zd,\"pwrite_errno\":%d,\"original_size\":%lld,\"original_length_restored\":%s}\n", n, error, (long long)original_size, restored == 0 && synced == 0 && closed == 0 ? "true" : "false");
        return n < 0 && (error == ENOSPC || error == EDQUOT) && restored == 0 && synced == 0 && closed == 0 ? 0 : 1;
    }
    return 64;
}
