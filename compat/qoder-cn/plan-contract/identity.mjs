// Synthetic test keys only. No user credentials or network access.
import { generateKeyPairSync } from 'node:crypto';
const { publicKey, privateKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
process.stdout.write(JSON.stringify({
  jwk: { ...publicKey.export({ format: 'jwk' }), kid: 'synthetic-signing-key', use: 'sig', alg: 'RS256' },
  pkcs8: privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64'),
}));
