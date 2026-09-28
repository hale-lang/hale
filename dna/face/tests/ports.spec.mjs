// The launchers' ports and readiness (the read API's #1220 shape, for the
// face's harnesses): a head's reads and its api binding's commands take
// ports chosen together, never the same one, and a head is ready only once
// it has announced the commands port it was given.
import { test, expect } from '@playwright/test';
import { freePorts, commandsAnnounced } from './command-wire.mjs';

test('ports chosen together are distinct', async () => {
  for (let i = 0; i < 50; i++) {
    const [reads, commands] = await freePorts(2);
    expect(reads).not.toBe(commands);
  }
});

test('a head is ready only once it has announced the commands port it was given', () => {
  const started = 'hale dna api: http://127.0.0.1:41551/?token=t  (trusted local)\n';
  const failed = started + 'lotus_tcp_listen_socket: bind: Address already in use\napi: the api binding\'s HTTP transport could not listen on 127.0.0.1:41551...\n';
  expect(commandsAnnounced(started, 41552)).toBe(false);
  expect(commandsAnnounced(failed, 41552)).toBe(false);
  expect(commandsAnnounced(started + 'hale dna api: commands http://127.0.0.1:41552/\n', 41552)).toBe(true);
  expect(commandsAnnounced(started + 'hale dna api: commands http://127.0.0.1:41553/\n', 41552)).toBe(false);
  expect(commandsAnnounced('', 0)).toBe(true);
});
