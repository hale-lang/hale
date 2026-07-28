CC ?= cc
CFLAGS ?= -O2 -g -Wall -Wextra -std=c11
LDFLAGS = -lpthread -lrt

all: synth peek

synth: synth.c protocol.h
	$(CC) $(CFLAGS) -o $@ synth.c $(LDFLAGS)

peek: peek.c protocol.h
	$(CC) $(CFLAGS) -o $@ peek.c $(LDFLAGS)

clean:
	rm -f synth peek *.dump

.PHONY: all clean
