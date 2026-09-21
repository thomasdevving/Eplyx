#!/usr/bin/env python3
"""Transport controls use simulated HTTP/curl responses, not production proof."""
import copy
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

import historical_transport as t

PARAMS = ['4zh6bmb77qX2CL7t5AJYCqa6YqFafbz3QJNeFvZjLowg',
          {'slot': 448195165, 'encoding': 'base64', 'commitment': 'finalized'}]
GOOD = {'jsonrpc': '2.0', 'id': 1, 'result': {'context': {'slot': 448195165}, 'value': {'example': True}}}


def wire(body=b'', status=200, exit_code=0, extra_headers=b'', stderr=b''):
    def run(args, **kwargs):
        Path(args[args.index('--output') + 1]).write_bytes(body)
        Path(args[args.index('--dump-header') + 1]).write_bytes(b'HTTP/2 ' + str(status).encode() + b'\r\nContent-Type: application/json\r\n' + extra_headers + b'\r\n')
        return types.SimpleNamespace(returncode=exit_code, stdout=str(status).encode(), stderr=stderr)
    return run


class TransportTests(unittest.TestCase):
    def test_failure_classes_are_distinct(self):
        cases = [(6, 0, b'', 'dns_failure'), (7, 0, b'', 'connect_failure'), (60, 0, b'', 'tls_failure'),
                 (28, 0, b'', 'timeout'), (18, 200, b'x', 'truncated_body'), (0, 429, b'', 'rate_limited'),
                 (0, 503, b'', 'gateway_failure'), (0, 403, b'', 'http_status'),
                 (0, 200, b'', 'empty_http_body'), (52, 0, b'', 'unknown_transport_failure'),
                 (0, 200, b'{bad', 'invalid_json'), (0, 200, b'{"error":{"code":-1}}', 'json_rpc_error')]
        for code, status, body, expected in cases:
            with self.subTest(expected=expected):
                self.assertEqual(t.classify(code, status, body, {})[0], expected)

    def test_empty_response_is_never_missing_account(self):
        failure, value = t.classify(0, 200, b'', {})
        self.assertEqual(failure, 'empty_http_body')
        self.assertIsNone(value)

    def test_diagnostic_fields_and_safe_headers(self):
        body = t.lut.canonical(GOOD)
        with patch.object(t.subprocess, 'run', wire(body, extra_headers=b'Content-Length: ' + str(len(body)).encode() + b'\r\nX-Request-ID: public-trace\r\nAuthorization: hidden\r\nSet-Cookie: hidden\r\n')):
            _, value, record = t.CurlArchive('https://archive.example/v2/private-path-token').once('getAccountInfo', PARAMS, 20)
        self.assertEqual(value, GOOD)
        for key in ('started_at_utc', 'elapsed_seconds', 'process_exit_code', 'http_status', 'response_headers',
                    'content_type', 'content_length_header', 'body_bytes', 'body_sha256', 'body_empty',
                    'provider_trace_ids', 'stderr', 'json_rpc_error', 'provider'):
            self.assertIn(key, record)
        self.assertEqual(record['provider_trace_ids'], {'x-request-id': 'public-trace'})
        self.assertNotIn('authorization', record['response_headers'])
        self.assertNotIn('set-cookie', record['response_headers'])

    def test_no_credential_url_or_echo_is_persisted(self):
        secret = 'UNIQUE_PRIVATE_SENTINEL'
        url = 'https://archive.example/v2/' + secret + '?key=' + secret
        body = t.lut.canonical({'error': {'code': -1, 'message': url}})
        with patch.object(t.subprocess, 'run', wire(body, stderr=('request ' + url + '\nAuthorization: Bearer ' + secret).encode(),
                                                  extra_headers=('X-Request-ID: ' + secret + '\r\n').encode())):
            _, _, record = t.CurlArchive(url).once('getAccountInfo', PARAMS, 20)
        encoded = t.lut.canonical(record).decode()
        self.assertNotIn(secret, encoded)
        self.assertNotIn('/v2/', encoded)
        self.assertNotIn('?key=', encoded)
        self.assertTrue(record['response_withheld'])
        self.assertEqual(record['provider'], 'https://archive.example')

    def capture(self, root, responses, validate=None):
        calls, delays = [], []
        rpc = t.CurlArchive('https://archive.example')
        def run(args, **kwargs):
            payload = json.loads(args[args.index('--data') + 1])
            calls.append(payload)
            self.assertLessEqual(len(calls), 5, 'unbounded retry reached watchdog')
            response = responses[min(len(calls) - 1, len(responses) - 1)]
            return wire(**response)(args, **kwargs)
        client = t.EvidenceClient(root, rpc, t.lut.read(t.POLICY_PATH), {}, delays.append)
        with patch.object(t.subprocess, 'run', run):
            value, facts, failure = client.call('getAccountInfo', copy.deepcopy(PARAMS), validate)
        client.finish({'failure': failure})
        return client, calls, delays, failure

    def test_four_exact_attempts_with_no_current_or_provider_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            client, calls, delays, failure = self.capture(Path(tmp) / 'attempt', [{'body': b''}])
            self.assertEqual(len(calls), 4)
            self.assertEqual(delays, [2, 5, 10])
            self.assertEqual(failure, 'empty_http_body')
            self.assertTrue(all(c['params'] == PARAMS for c in calls))
            self.assertTrue(all(a['provider'] == 'https://archive.example' for a in client.receipt['attempts']))
            t.verify(client.root, [('getAccountInfo', PARAMS)])

    def test_later_success_keeps_failed_attempt_provenance(self):
        with tempfile.TemporaryDirectory() as tmp:
            client, calls, _, failure = self.capture(Path(tmp) / 'attempt', [{'body': b''}, {'body': t.lut.canonical(GOOD)}])
            self.assertIsNone(failure)
            self.assertEqual(len(calls), 2)
            self.assertEqual(len(client.receipt['attempts']), 2)
            first = client.receipt['attempts'][0]
            self.assertEqual(first['failure_class'], 'empty_http_body')
            self.assertEqual((client.root / first['body_file']).read_bytes(), b'')
            t.verify(client.root, [('getAccountInfo', PARAMS)])

    def test_slot_contradiction_and_missing_account_not_retried(self):
        for result, expected in [({'context': {'slot': 448195166}, 'value': {}}, 'slot_context_mismatch'),
                                 ({'context': {'slot': 448195165}, 'value': None}, 'missing_account')]:
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as tmp:
                _, calls, _, failure = self.capture(Path(tmp) / 'attempt', [{'body': t.lut.canonical({'result': result})}])
                self.assertEqual(len(calls), 1)
                self.assertEqual(failure, expected)

    def test_contradictory_account_not_retried_or_ignored(self):
        def reject(_result):
            raise ValueError('wrong historical owner')
        with tempfile.TemporaryDirectory() as tmp:
            client, calls, _, failure = self.capture(Path(tmp) / 'attempt', [{'body': t.lut.canonical(GOOD)}], reject)
            self.assertEqual(len(calls), 1)
            self.assertEqual(failure, 'contradictory_account')
            t.verify(client.root, [('getAccountInfo', PARAMS)], {t.request_id('getAccountInfo', PARAMS): reject})

    def test_offline_verifier_rejects_changed_slot_provider_or_hash(self):
        for change in ('slot', 'provider', 'hash', 'attempt'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as tmp:
                client, _, _, _ = self.capture(Path(tmp) / 'attempt', [{'body': t.lut.canonical(GOOD)}])
                a = client.receipt['attempts'][0]
                if change == 'slot':
                    a['params'][1]['slot'] += 1
                elif change == 'provider':
                    a['provider'] = 'https://other.example'
                elif change == 'hash':
                    a['body_sha256'] = '0' * 64
                else:
                    a['attempt'] = 5
                client.save()
                with self.assertRaises(ValueError):
                    t.verify(client.root, [('getAccountInfo', PARAMS)])

    def test_current_request_rejected_before_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            client = t.EvidenceClient(Path(tmp) / 'attempt', t.CurlArchive('https://archive.example'), t.lut.read(t.POLICY_PATH), {})
            params = copy.deepcopy(PARAMS)
            del params[1]['slot']
            with self.assertRaisesRegex(ValueError, 'exact historical context'), patch.object(t.subprocess, 'run', side_effect=AssertionError('network forbidden')):
                client.call('getAccountInfo', params)


if __name__ == '__main__':
    unittest.main()
