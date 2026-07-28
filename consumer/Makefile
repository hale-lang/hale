CC ?= cc
CFLAGS ?= -O2 -g -Wall -Wextra -std=c11
LDFLAGS = -lpthread -lrt

all: fuse

fuse: fuse.c obs_attach.c obs_attach.h ../emitter/protocol.h
	$(CC) $(CFLAGS) -o $@ fuse.c obs_attach.c $(LDFLAGS)

clean:
	rm -f fuse

.PHONY: all clean
