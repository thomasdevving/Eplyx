#!/usr/bin/env python3
"""Observable, bounded exact-context archive transport; no replay admission."""
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import tempfile
import time
from urllib.parse import urlsplit, unquote, parse_qsl

import kamino_u3b_lut as lut

POLICY_PATH = lut.REPO / 'docs/examples/phase-u3e-validation/transport-policy.json'
SAFE_HEADERS = {'content-type', 'content-length', 'content-encoding', 'date', 'server',
                'retry-after', 'x-request-id', 'x-correlation-id', 'x-amzn-trace-id', 'cf-ray', 'traceparent'}
TRACE_HEADERS = {'x-request-id', 'x-correlation-id', 'x-amzn-trace-id', 'cf-ray', 'traceparent'}


def identity(url):
    parsed = urlsplit(url)
    lut.require(parsed.scheme in ('http', 'https') and parsed.hostname and not parsed.username
                and not parsed.password and not any(c in url for c in '\r\n"\\'), 'invalid provider configuration')
    return f'{parsed.scheme}://{parsed.hostname}'


def scrub(text, secrets=()):
    for secret in sorted(set(secrets), key=len, reverse=True):
        if secret:
            text = text.replace(secret, '[redacted]')
    def host_only(match):
        try:
            parsed = urlsplit(match[0])
            return f'{parsed.scheme}://{parsed.hostname}' if parsed.hostname else '[redacted-url]'
        except ValueError:
            return '[redacted-url]'
    text = re.sub(r'https?://[^\s"<>]+', host_only, text)
    text = re.sub(r'(?im)(authorization|proxy-authorization|(?:x-)?api[-_]?key|access[-_]?token|cookie|set-cookie)\s*[:=]\s*[^\r\n]*', r'\1: [redacted]', text)
    text = re.sub(r'(?i)Bearer\s+\S+', 'Bearer [redacted]', text)
    return text


def headers(raw, secrets=()):
    # Keep only the final HTTP response block, not a proxy CONNECT response.
    selected = {}
    for line in raw.decode('latin1').splitlines():
        if line.startswith('HTTP/'):
            selected = {}
        elif ':' in line:
            key, value = line.split(':', 1)
            key = key.strip().lower()
            if key in SAFE_HEADERS:
                selected[key] = scrub(value.strip(), secrets)
    return selected


def scrub_value(value, secrets=()):
    if isinstance(value, dict):
        return {scrub(str(k), secrets): '[redacted]' if re.search(r'authorization|api.?key|secret|access.?token|cookie', str(k), re.I)
                else scrub_value(v, secrets) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub_value(v, secrets) for v in value]
    return scrub(value, secrets) if isinstance(value, str) else value


def classify(exit_code, http_status, body, response_headers):
    if exit_code == 6:
        return 'dns_failure', None
    if exit_code == 7:
        return 'connect_failure', None
    if exit_code in (35, 51, 53, 58, 59, 60, 64, 66, 77, 80, 82, 83, 90, 91):
        return 'tls_failure', None
    if exit_code == 28:
        return 'timeout', None
    if exit_code == 18:
        return 'truncated_body', None
    if http_status == 429:
        return 'rate_limited', None
    if http_status in (502, 503, 504):
        return 'gateway_failure', None
    if http_status and not 200 <= http_status < 300:
        return 'http_status', None
    if exit_code != 0 or not http_status:
        return 'unknown_transport_failure', None
    if not body:
        return 'empty_http_body', None
    length = response_headers.get('content-length')
    if length and length.isdigit() and int(length) != len(body):
        return 'truncated_body', None
    try:
        value = json.loads(body)
    except (ValueError, UnicodeError):
        return 'invalid_json', None
    if not isinstance(value, dict) or ('result' not in value and 'error' not in value):
        return 'invalid_json', None
    if value.get('error') is not None:
        return 'json_rpc_error', value
    return None, value


class CurlArchive:
    def __init__(self, endpoint, origin=''):
        self.provider = identity(endpoint)
        lut.require(not any(c in origin for c in '\r\n"\\'), 'invalid Origin configuration')
        self.endpoint, self.origin = endpoint, origin
        parsed = urlsplit(endpoint)
        self.secrets = [unquote(v) for _, v in parse_qsl(parsed.query)]
        self.secrets += [part for part in parsed.path.split('/') if part and part not in ('v1', 'v2', 'v3', 'rpc', 'api')]

    def once(self, method, params, timeout):
        config = f'url = "{self.endpoint}"\nheader = "Content-Type: application/json"\n'
        if self.origin:
            config += f'header = "Origin: {self.origin}"\n'
        started = datetime.now(timezone.utc).isoformat()
        tick = time.perf_counter()
        with tempfile.TemporaryDirectory(prefix='eplyx-transport-') as tmp:
            body_file, header_file = Path(tmp) / 'body', Path(tmp) / 'headers'
            try:
                process = subprocess.run(['curl', '--config', '-', '--silent', '--show-error',
                    '--max-time', str(timeout), '--max-redirs', '0', '--request', 'POST',
                    '--data', json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params}),
                    '--output', str(body_file), '--dump-header', str(header_file),
                    '--write-out', '%{http_code}'], input=config.encode(), capture_output=True, timeout=timeout + 5)
                exit_code, stdout, stderr = process.returncode, process.stdout, process.stderr
                status = int(stdout.strip()) if stdout.strip().isdigit() else None
                timed_out = False
            except subprocess.TimeoutExpired as exc:
                exit_code, status, stderr, timed_out = None, None, exc.stderr or b'', True
            body = body_file.read_bytes() if body_file.exists() else b''
            raw_headers = header_file.read_bytes() if header_file.exists() else b''
        safe_headers = headers(raw_headers, self.secrets)
        failure, value = classify(28 if timed_out else exit_code, status, body, safe_headers)
        safe_stderr = scrub(stderr.decode('utf8', errors='replace'), self.secrets)
        rpc_error = value.get('error') if value else None
        if rpc_error is not None:
            rpc_error = scrub_value(rpc_error, self.secrets)
        record = {'started_at_utc': started, 'elapsed_seconds': time.perf_counter() - tick,
                  'process_exit_code': exit_code, 'process_timeout': timed_out,
                  'http_status': status, 'response_headers': safe_headers,
                  'content_type': safe_headers.get('content-type'),
                  'content_length_header': safe_headers.get('content-length'),
                  'body_bytes': len(body), 'body_sha256': lut.sha(body), 'body_empty': len(body) == 0,
                  'provider_trace_ids': {k: v for k, v in safe_headers.items() if k in TRACE_HEADERS},
                  'stderr': safe_stderr, 'json_rpc_error': rpc_error, 'failure_class': failure,
                  'provider': self.provider, 'response_withheld': False}
        try:
            safe_value = json.loads(body) if body else ''
        except (ValueError, UnicodeError):
            safe_value = body.decode('utf8', errors='replace')
        try:
            lut.baseline.hygiene(safe_value)
            lut.require(not any(s.encode() in body for s in self.secrets if s), 'credential echo')
        except ValueError:
            record.update(response_withheld=True, failure_class='unknown_transport_failure', withholding_reason='unsafe_response_withheld')
            value = None
        lut.baseline.hygiene(record)
        return body, value, record


def request_id(method, params):
    return lut.sha(lut.canonical([method, params]))


class EvidenceClient:
    def __init__(self, root, transport, policy, qualification, sleep=time.sleep):
        lut.require(not root.exists(), 'one immutable directory per attempt')
        self.root, self.transport, self.policy, self.sleep = root, transport, policy, sleep
        self.provider = transport.provider
        root.mkdir(parents=True)
        self.receipt = {'kind': 'u3e_diagnostic_transport', 'complete': False, 'provider': self.provider,
                        'qualification': qualification, 'policy': policy, 'attempts': []}
        self.save()

    def save(self):
        (self.root / 'receipt.json').write_bytes(lut.canonical(self.receipt))

    def call(self, method, params, validate=None, allow_absence=False):
        lut.require(not allow_absence or validate is not None, 'absence requires an explicit boundary validator')
        lut.require(method in ('getAccountInfo', 'getGenesisHash', 'getBlock'), 'unsupported archive method')
        if method == 'getAccountInfo':
            lut.require(type(params[1].get('slot')) is int and params[1]['commitment'] == 'finalized'
                        and params[1]['encoding'] == 'base64', 'exact historical context required; no current fallback')
        exact = lut.canonical(params)
        key = request_id(method, params)
        lut.require(not any(a['logical_request_id'] == key for a in self.receipt['attempts']), 'exact context already attempted; no hidden retry reset')
        for number, delay in enumerate(self.policy['backoff_seconds'], 1):
            lut.require(number <= self.policy['max_attempts_per_context'], 'retry bound exceeded')
            if delay:
                self.sleep(delay)
            lut.require(lut.canonical(params) == exact and self.transport.provider == self.provider, 'request context/provider changed')
            body, value, record = self.transport.once(method, params, self.policy['request_timeout_seconds'])
            lut.require(record['provider'] == self.provider, 'provider switch rejected')
            ref = None if record['response_withheld'] else f'rpc/{key}-{number}.body'
            record.update(logical_request_id=key, method=method, params=json.loads(exact),
                          account=params[0] if method == 'getAccountInfo' else None,
                          requested_slot=params[1]['slot'] if method == 'getAccountInfo' else params[0] if method == 'getBlock' else None,
                          attempt=number, backoff_seconds=delay, body_file=ref, validation_failure=None)
            if ref:
                (self.root / 'rpc').mkdir(exist_ok=True)
                (self.root / ref).write_bytes(body)
            result = value.get('result') if value else None
            facts = None
            if record['failure_class'] is None:
                if method == 'getAccountInfo' and (not isinstance(result, dict) or result.get('context', {}).get('slot') != params[1]['slot']):
                    record['failure_class'] = 'slot_context_mismatch'
                elif method == 'getAccountInfo' and result.get('value') is None and not allow_absence:
                    record['failure_class'] = 'missing_account'
                elif validate:
                    try:
                        facts = validate(result)
                    except (ValueError, KeyError, TypeError) as exc:
                        record.update(failure_class='contradictory_account', validation_failure=scrub(str(exc)))
            self.receipt['attempts'].append(record)
            self.save()  # Failed attempts survive later success.
            failure = record['failure_class']
            if failure is None:
                return result, facts, None
            if failure not in self.policy['retryable'] or record['response_withheld']:
                break
        return None, None, failure

    def finish(self, result):
        (self.root / 'result.json').write_bytes(lut.canonical(result))
        self.receipt['complete'] = True
        self.save()


def verify(root, expected_contexts, validators=None, allow_absence=()):
    """Reclassify retained wire evidence. Never instantiate an RPC client."""
    receipt = lut.read(root / 'receipt.json')
    policy = lut.read(POLICY_PATH)
    lut.require(receipt['complete'] and receipt['policy'] == policy, 'transport policy or completion differs')
    lut.require(identity(receipt['provider']) == receipt['provider'], 'full/credential-bearing provider URL persisted')
    lut.baseline.hygiene(receipt)
    allowed = {request_id(method, params): (method, params) for method, params in expected_contexts}
    context_order = {key: index for index, key in enumerate(allowed)}
    validators = validators or {}
    grouped, files = {}, set()
    previous_context, previous_end = -1, None
    for attempt in receipt['attempts']:
        key = request_id(attempt['method'], attempt['params'])
        lut.require(key == attempt['logical_request_id'] and key in allowed, 'exact request context changed')
        lut.require(attempt['provider'] == receipt['provider'], 'provider switch rejected')
        lut.require(isinstance(attempt['started_at_utc'], str)
                    and datetime.fromisoformat(attempt['started_at_utc']).utcoffset() is not None
                    and isinstance(attempt['elapsed_seconds'], (float, int)) and attempt['elapsed_seconds'] >= 0,
                    'invalid transport timing')
        start = datetime.fromisoformat(attempt['started_at_utc']).timestamp()
        lut.require(previous_end is None or start >= previous_end, 'overlapping or reordered sequential requests')
        previous_end = start + attempt['elapsed_seconds']
        lut.require(context_order[key] >= previous_context, 'request context order or interleaved retries differs')
        previous_context = context_order[key]
        lut.require(attempt['content_type'] == attempt['response_headers'].get('content-type')
                    and attempt['content_length_header'] == attempt['response_headers'].get('content-length')
                    and attempt['provider_trace_ids'] == {k: v for k, v in attempt['response_headers'].items() if k in TRACE_HEADERS},
                    'transport header diagnostics differ')
        method, params = allowed[key]
        lut.require(attempt['account'] == (params[0] if method == 'getAccountInfo' else None)
                    and attempt['requested_slot'] == (params[1]['slot'] if method == 'getAccountInfo' else params[0] if method == 'getBlock' else None),
                    'request diagnostic identity differs')
        group = grouped.setdefault(key, [])
        number = len(group) + 1
        lut.require(attempt['attempt'] == number <= policy['max_attempts_per_context'], 'retry bound or attempt membership differs')
        lut.require(attempt['backoff_seconds'] == policy['backoff_seconds'][number - 1], 'backoff policy differs')
        if group:
            lut.require(group[-1]['failure_class'] in policy['retryable'] and not group[-1]['response_withheld'], 'nontransport/contradictory evidence retried')
        ref = attempt['body_file']
        if ref:
            lut.require(ref == f'rpc/{key}-{number}.body' and ref not in files, 'body membership differs')
            files.add(ref)
            body = lut.safe_file(root, ref).read_bytes()
            lut.require(lut.sha(body) == attempt['body_sha256'] and len(body) == attempt['body_bytes']
                        and (len(body) == 0) == attempt['body_empty'], 'raw body hash/length differs')
            failure, value = classify(28 if attempt['process_timeout'] else attempt['process_exit_code'],
                                      attempt['http_status'], body, attempt['response_headers'])
            result = value.get('result') if value else None
            validation_failure = None
            if failure is None:
                method, params = allowed[key]
                if method == 'getAccountInfo' and (not isinstance(result, dict) or result.get('context', {}).get('slot') != params[1]['slot']):
                    failure = 'slot_context_mismatch'
                elif method == 'getAccountInfo' and result.get('value') is None and key not in allow_absence:
                    failure = 'missing_account'
                elif key in validators:
                    try:
                        validators[key](result)
                    except (ValueError, KeyError, TypeError) as exc:
                        failure, validation_failure = 'contradictory_account', scrub(str(exc))
            lut.require(failure == attempt['failure_class'] and validation_failure == attempt['validation_failure'], 'failure classification differs')
        else:
            lut.require(attempt['response_withheld'] and attempt['failure_class'] == 'unknown_transport_failure', 'missing raw response')
        lut.require(set(attempt['response_headers']) <= SAFE_HEADERS, 'unsafe headers retained')
        group.append(attempt)
    lut.require(files == {str(p.relative_to(root)) for p in (root / 'rpc').glob('*')}, 'raw file membership differs')
    return grouped
