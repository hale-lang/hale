/* lotus-wasm POSIX declaration stubs (WASM plan, Phase 1).
 *
 * The browser has no sockets / fs / processes / terminals / epoll /
 * ucontext coroutines / OS threads. But lotus_arena.c is one
 * translation unit, so the functions implementing those (gated-out)
 * features must still COMPILE for wasm. This header supplies just
 * enough — syscall declarations, POSIX constants, and struct stubs
 * (correct field NAMES; layout is irrelevant) — for them to compile.
 * They are never reached from a wasm program's compute/arena/bus path,
 * so `wasm-ld --gc-sections` strips every one of them from the final
 * module. Nothing here runs; it exists only to satisfy the compiler.
 *
 * Included from lotus_wasm_shim.h. */
#ifndef LOTUS_WASM_POSIX_H
#define LOTUS_WASM_POSIX_H

typedef long ssize_t;

/* ---- fd / std streams --------------------------------------------- */
#define STDIN_FILENO  0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2
ssize_t read(int, void *, size_t);
ssize_t write(int, const void *, size_t);
int close(int);
int open(const char *, int, ...);
int fcntl(int, int, ...);
int ioctl(int, unsigned long, ...);
int pipe(int[2]);
int dup2(int, int);
off_t lseek(int, off_t, int);
int unlink(const char *);
int rename(const char *, const char *);
int mkdir(const char *, unsigned);
int mkstemps(char *, int);
void perror(const char *);
#define O_RDONLY   0
#define O_WRONLY   1
#define O_RDWR     2
#define O_CREAT    0100
#define O_TRUNC    01000
#define O_APPEND   02000
#define O_NONBLOCK 04000
#define F_GETFL    3
#define F_SETFL    4
#define SEEK_SET   0
#define SEEK_CUR   1
#define SEEK_END   2
__attribute__((noreturn)) void exit(int);
__attribute__((noreturn)) void _exit(int);

/* string scan + dup (real impls in lotus_wasm_libc.c — used by the
 * json/bytes core, so these must work, not stub). */
void  *memchr(const void *, int, size_t);
char  *strchr(const char *, int);
char  *strrchr(const char *, int);
char  *strstr(const char *, const char *);
char  *strdup(const char *);

/* ---- stdio (FILE* paths beyond the inert diag set in the shim) ---- */
FILE *fopen(const char *, const char *);
char *fgets(char *, int, FILE *);
int   feof(FILE *);
int   setvbuf(FILE *, char *, int, size_t);
ssize_t getline(char **, size_t *, FILE *);
extern FILE *const stdin;
#define _IOLBF 1

/* ---- sockets ------------------------------------------------------ */
struct in_addr { unsigned int s_addr; };
struct sockaddr { unsigned short sa_family; char sa_data[14]; };
struct sockaddr_in { unsigned short sin_family; unsigned short sin_port; struct in_addr sin_addr; char sin_zero[8]; };
struct sockaddr_un { unsigned short sun_family; char sun_path[108]; };
struct addrinfo {
    int ai_flags, ai_family, ai_socktype, ai_protocol;
    socklen_t ai_addrlen; struct sockaddr *ai_addr;
    char *ai_canonname; struct addrinfo *ai_next;
};
struct iovec { void *iov_base; size_t iov_len; };
struct msghdr {
    void *msg_name; socklen_t msg_namelen;
    struct iovec *msg_iov; size_t msg_iovlen;
    void *msg_control; size_t msg_controllen; int msg_flags;
};
struct cmsghdr { size_t cmsg_len; int cmsg_level; int cmsg_type; };
#define CMSG_SPACE(n) (((n) + sizeof(struct cmsghdr) + 7u) & ~7u)
int socket(int, int, int);
int bind(int, const struct sockaddr *, socklen_t);
int listen(int, int);
int accept(int, struct sockaddr *, socklen_t *);
int connect(int, const struct sockaddr *, socklen_t);
int shutdown(int, int);
int setsockopt(int, int, int, const void *, socklen_t);
int getsockopt(int, int, int, void *, socklen_t *);
int getsockname(int, struct sockaddr *, socklen_t *);
ssize_t send(int, const void *, size_t, int);
ssize_t recv(int, void *, size_t, int);
ssize_t sendto(int, const void *, size_t, int, const struct sockaddr *, socklen_t);
ssize_t recvfrom(int, void *, size_t, int, struct sockaddr *, socklen_t *);
ssize_t recvmsg(int, struct msghdr *, int);
int getaddrinfo(const char *, const char *, const struct addrinfo *, struct addrinfo **);
void freeaddrinfo(struct addrinfo *);
const char *gai_strerror(int);
int inet_pton(int, const char *, void *);
const char *inet_ntop(int, const void *, char *, socklen_t);
struct ip_mreq { struct in_addr imr_multiaddr; struct in_addr imr_interface; };
unsigned int   htonl(unsigned int);
unsigned short htons(unsigned short);
unsigned int   ntohl(unsigned int);
unsigned short ntohs(unsigned short);
#define AF_UNIX  1
#define AF_INET  2
#define SOCK_STREAM 1
#define SOCK_DGRAM  2
#define SOCK_SEQPACKET 5
#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#define SO_REUSEPORT 15
#define SO_KEEPALIVE 9
#define SO_BROADCAST 6
#define SO_LINGER 13
#define SO_PRIORITY 12
#define SO_RCVBUF 8
#define SO_SNDBUF 7
#define SO_RCVTIMEO 20
#define SO_SNDTIMEO 21
#define IPPROTO_IP 0
#define IPPROTO_TCP 6
#define IPPROTO_UDP 17
#define IPPROTO_IPV6 41
#define TCP_NODELAY 1
#define IP_TTL 2
#define IP_TOS 1
#define IP_PKTINFO 8
#define IP_MULTICAST_IF 32
#define IP_MULTICAST_TTL 33
#define IP_MULTICAST_LOOP 34
#define IP_ADD_MEMBERSHIP 35
#define IP_DROP_MEMBERSHIP 36
#define INADDR_ANY 0
#define SHUT_RDWR 2
#define EAI_NONAME -2

/* ---- poll / epoll / eventfd --------------------------------------- */
struct pollfd { int fd; short events; short revents; };
typedef unsigned long nfds_t;
int poll(struct pollfd *, nfds_t, int);
#define POLLIN  0x001
#define POLLERR 0x008
#define POLLHUP 0x010
typedef union { void *ptr; int fd; unsigned u32; unsigned long u64; } epoll_data_t;
struct epoll_event { unsigned int events; epoll_data_t data; };
int epoll_create1(int);
int epoll_ctl(int, int, int, struct epoll_event *);
int epoll_wait(int, struct epoll_event *, int, int);
int eventfd(unsigned, int);
#define EPOLLIN  0x001
#define EPOLLOUT 0x004
#define EPOLL_CTL_ADD 1
#define EPOLL_CTL_DEL 2
#define EPOLL_CLOEXEC 02000000
#define EFD_CLOEXEC 02000000
#define EFD_NONBLOCK 04000

/* ---- stat / dir --------------------------------------------------- */
struct stat { off_t st_size; unsigned int st_mode; };
int stat(const char *, struct stat *);
int fstat(int, struct stat *);
typedef struct __lotus_DIR DIR;
struct dirent { char d_name[256]; };
DIR *opendir(const char *);
struct dirent *readdir(DIR *);
void rewinddir(DIR *);
int closedir(DIR *);

/* ---- process / signal --------------------------------------------- */
int fork(void);
int execvp(const char *, char *const[]);
int waitpid(int, int *, int);
#define WNOHANG 1
int kill(int, int);
int setpgid(int, int);
void (*signal(int, void (*)(int)))(int);
#define SIG_IGN  ((void (*)(int))1)
#define SIGPIPE 13
#define SIGTERM 15
#define SIGKILL 9
#define WIFEXITED(s)   (((s) & 0x7f) == 0)
#define WEXITSTATUS(s) (((s) >> 8) & 0xff)
#define WIFSIGNALED(s) (((s) & 0x7f) != 0 && ((s) & 0x7f) != 0x7f)
#define WTERMSIG(s)    ((s) & 0x7f)

/* ---- termios ------------------------------------------------------ */
struct termios { tcflag_t c_iflag, c_oflag, c_cflag, c_lflag; unsigned char c_cc[32]; };
struct winsize { unsigned short ws_row, ws_col, ws_xpixel, ws_ypixel; };
int tcgetattr(int, struct termios *);
int tcsetattr(int, int, const struct termios *);
int isatty(int);
#define TCSAFLUSH 2
#define VMIN  6
#define VTIME 5
#define BRKINT 0002
#define ICRNL  0400
#define INPCK  020
#define ISTRIP 040
#define IXON   02000
#define OPOST  01
#define ECHO   010
#define ICANON 02
#define IEXTEN 0100000
#define ISIG   01
#define CS8    060
#define TIOCGWINSZ 0x5413

/* ---- threads / scheduling ----------------------------------------- */
int pthread_create(pthread_t *, const pthread_attr_t *, void *(*)(void *), void *);
int pthread_join(pthread_t, void **);
int pthread_cond_init(pthread_cond_t *, const pthread_condattr_t *);
int pthread_cond_destroy(pthread_cond_t *);
int pthread_cond_wait(pthread_cond_t *, pthread_mutex_t *);
int pthread_cond_signal(pthread_cond_t *);
int pthread_cond_broadcast(pthread_cond_t *);
int sched_yield(void);
#define PTHREAD_COND_INITIALIZER {0}
typedef struct { unsigned long __bits[16]; } cpu_set_t;
#define CPU_ZERO(s) ((void)(s))
#define CPU_SET(c, s) ((void)(s))
int pthread_setaffinity_np(pthread_t, size_t, const cpu_set_t *);

/* ---- ucontext coroutines (async_io; gated-out feature) ------------ */
typedef struct { void *ss_sp; int ss_flags; size_t ss_size; } stack_t;
typedef struct __lotus_ucontext {
    struct __lotus_ucontext *uc_link;
    stack_t uc_stack;
    void *__opaque[32];
} ucontext_t;
int  getcontext(ucontext_t *);
void setcontext(const ucontext_t *);
void makecontext(ucontext_t *, void (*)(void), int, ...);
int  swapcontext(ucontext_t *, const ucontext_t *);

/* ---- time formatting / rusage / mlock ----------------------------- */
struct tm { int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday, tm_isdst; };
struct tm *gmtime_r(const time_t *, struct tm *);
size_t strftime(char *, size_t, const char *, const struct tm *);
struct rusage { struct timeval ru_utime, ru_stime; long ru_maxrss; };
int getrusage(int, struct rusage *);
#define RUSAGE_SELF 0
int mlockall(int);
#define MCL_CURRENT 1
#define MCL_FUTURE  2

#endif /* LOTUS_WASM_POSIX_H */
