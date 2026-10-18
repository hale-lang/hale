import socket, sys, time, http.client, json
N = int(sys.argv[2]); mode = sys.argv[1]
lat = []
if mode == 'unix':
    s = socket.socket(socket.AF_UNIX); s.connect('/tmp/hale-bench-api.sock')
    f = s.makefile('rb')
    for i in range(N):
        line = ('{"call":"Echo::ping","payload":{"n":%d,"tag":"abc"},"id":"c"}\n' % i).encode()
        t = time.perf_counter_ns(); s.sendall(line); r = f.readline(); lat.append(time.perf_counter_ns() - t)
        if i == 0 and b'"n":1' not in r: print("bad", r); sys.exit(1)
else:
    c = http.client.HTTPConnection('127.0.0.1', 18431)
    for i in range(N):
        body = '{"n":%d,"tag":"abc"}' % i
        t = time.perf_counter_ns(); c.request('POST', '/call/Echo::ping', body, {'Content-Type': 'application/json', 'Authorization': 'Bearer x'}); r = c.getresponse(); d = r.read(); lat.append(time.perf_counter_ns() - t)
        if i == 0 and (r.status != 200 or b'"n":1' not in d): print("bad", r.status, d); sys.exit(1)
lat.sort()
print("%s p50=%.1fus p99=%.1fus" % (mode, lat[len(lat)//2]/1000, lat[len(lat)*99//100]/1000))
