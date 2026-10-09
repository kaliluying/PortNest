import net from 'node:net';

const ignoreTermination = process.argv.includes('--ignore-term');
const servers = [net.createServer(), net.createServer()];
if (ignoreTermination) process.on('SIGTERM', () => {});
await Promise.all(servers.map((server, index) => new Promise((resolve, reject) => {
  server.once('error', reject);
  server.listen({ host: index === 0 ? '127.0.0.1' : '::1', port: 0 }, resolve);
})));
process.stdout.write(JSON.stringify({ pid: process.pid, ports: servers.map(server => server.address().port), ignoreTermination }) + '\n');
