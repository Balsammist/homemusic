"""In-memory backend for owned desktop checks. Never imports production code/config."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import threading
import time
from urllib.parse import urlsplit, parse_qs, quote, unquote
import wave
import struct
import zlib

TRACKS = [dict(ref=f'nd:fixture-{i}', title=f'Fixture Track {i}', artist='Fixture Artist',
               album_title='Fixture Album', duration_s=45.0, nd_song_id=f'fixture-{i}',
               nd_album_id='fixture-album', nd_artist_id='fixture-artist', owned=True)
          for i in range(1, 7)]
WAVE_TRACKS = [{**TRACKS[0], 'ref': f'nd:wave-{i}', 'title': f'Wave Track {i}', 'nd_song_id': f'wave-{i}'} for i in range(40)]
LARGE_TRACKS = [
    {**TRACKS[i % 6], 'ref': f'nd:large-{i}', 'title': f'Artwork {i}',
     'cover_small_url': f'/fixture/cover.png?id={i}'}
    for i in range(300)
]
PLAYABLE_REFS = {track['ref'] for track in TRACKS + WAVE_TRACKS + LARGE_TRACKS}
PREVIEW_TRACK = dict(ref='dz:fixture-preview', title='Fixture Preview', artist='Fixture Artist')
UNAVAILABLE_TRACK = dict(ref='dz:fixture-missing', title='Fixture Missing', artist='Fixture Artist')

ALBUM = dict(id='fixture-album', title='Fixture Album', artist='Fixture Artist', artist_id='fixture-artist',
             cover_url='/fixture/cover.png?id=9001')
PLAYLIST = dict(id='fixture-playlist', name='Fixture Playlist', song_count=6)


class Fixture:
    def __init__(self):
        self.requests = []
        self.failures = set()
        self.delays = {}
        self.extra_audio = {}
        self.listen_stream = False
        self.lock = threading.Lock()
        buffer = io.BytesIO()
        with wave.open(buffer, 'wb') as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(16000)
            audio.writeframes(b'\0\0' * (16000 * 45))
        self.audio = buffer.getvalue()
        self.wave_number = 0
        self.download_item = {'id': 707, 'artist': 'Fixture Artist', 'title': 'Failed Download',
                              'status': 'failed', 'error': 'Fixture source unavailable', 'progress': 0}
        def chunk(kind,data):
            return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
        self.cover = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',32,32,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+bytes([120,80,190])*32)*32))+chunk(b'IEND',b'')

    def fail(self, path, enabled):
        with self.lock:
            if enabled:
                self.failures.add(path)
            else:
                self.failures.discard(path)

    def response(self, method, path, body, query):
        if method == 'POST' and path == '/api/auth/login':
            return (200, {'ok': True}) if body == {'username': 'fixture', 'password': 'fixture'} else (401, {})
        if method == 'POST' and path == '/api/downloads':
            return 200, {'id':'fixture-download'}
        if path in ('/api/downloads/707', '/api/downloads/items/707/retry'):
            with self.lock:
                if method == 'POST':
                    if self.download_item['status'] != 'failed':
                        return 409, {'detail': 'Duplicate retry'}
                    self.download_item.update(status='queued', error=None)
                return 200, dict(self.download_item)
        if method == 'POST' and path == '/api/listen/prefetch':
            return 200, {'ok': True}
        if method == 'POST' and path == '/api/wave/start':
            self.wave_number += 1
            return 200, {'session_id': f'fixture-wave-{self.wave_number}', 'tracks':WAVE_TRACKS[:5], 'pending':True}
        if method == 'POST' and path.startswith('/api/wave/') and path.endswith('/next'):
            excluded = set((body or {}).get('queued', []))
            return 200, {'session_id':path.split('/')[3], 'tracks':[t for t in WAVE_TRACKS if t['ref'] not in excluded][:5], 'pending':False}
        if method != 'GET':
            return 405, {'detail': 'Unexpected fixture mutation'}
        if path == '/api/listen':
            refs = query.get('ref', [])
            if len(refs) != 1 or refs[0] not in PLAYABLE_REFS:
                return 422, {'detail': 'Unknown fixture track reference'}
            return 200, {
                'stream_url': f'/api/listen/stream?ref={quote(refs[0], safe="")}' if self.listen_stream else '/fixture/audio.wav',
                'kind': 'library', 'provider': 'Fixture',
            }
        if path == '/api/search' and query.get('q') == ['all']:
            return 200, {'tracks':TRACKS,'library':[{'id':401,'nd_song_id':'fixture-1','title':'Fixture Track 1','artist':'Fixture Artist','album':'Fixture Album','duration_s':45}],'albums':[{'ref':'dz:fixture-album','title':'Search Album','artist':'Fixture Artist','year':2001}], 'artists':[{'ref':'dz:fixture-artist','name':'Search Artist','picture_url':'/fixture/cover.png?id=9000'}]}
        if path == '/api/search' and self.listen_stream:
            title = query.get('q', [''])[0].casefold()
            return 200, {'tracks': [track for track in [*TRACKS, PREVIEW_TRACK, UNAVAILABLE_TRACK]
                                    if title == track['title'].casefold()]}
        if method == 'GET' and path.startswith('/api/catalog/track/'):
            reference = unquote(path.rsplit('/', 1)[-1])
            track = next((t for t in [*TRACKS, PREVIEW_TRACK, UNAVAILABLE_TRACK] if t['ref'] == reference), TRACKS[0])
            return 200, {**track, 'ref': reference, 'cover_url': '/fixture/cover.png?id=0'}
        routes = {
            '/api/mixes/personal': {'groups': []},
            '/api/starred': {'refs': [x['ref'] for x in TRACKS], 'song_ids': [], 'subjects': []},
            '/api/nd/home': {'recent': [ALBUM], 'frequent': [], 'starred': []},
            '/api/nd/liked': {'songs': TRACKS},
            '/api/mixes/today': {'onboarding': False, 'mix': {'title': 'Fixture Mix', 'tracks': TRACKS}},
            '/api/plays/recent': {'tracks': []},
            '/api/nd/albums': [ALBUM],
            '/api/nd/albums/fixture-album': {**ALBUM, 'tracks': TRACKS},
            '/api/nd/artists': [{'id':'fixture-artist','name':'Fixture Artist','album_count':1}],
            '/api/nd/artists/fixture-artist': {'artist': {'id': 'fixture-artist', 'name': 'Fixture Artist'}, 'top_tracks': TRACKS, 'albums': [ALBUM]},
            '/api/playlists/links': {'name':'Metadata links','tracks':[{**TRACKS[0], 'nd_artist_id':None, 'artist_ref':None}, {**TRACKS[1], 'ref':'', 'nd_song_id':None,'nd_artist_id':None,'artist_ref':None,'artist':'Absent Artist','nd_album_id':None,'album_ref':None}]},
            '/api/playlists/large': {'name':'Large artwork fixture','tracks':LARGE_TRACKS},
            '/api/playlists': [PLAYLIST],
            '/api/playlists/fixture-playlist': {**PLAYLIST, 'tracks': TRACKS},
            '/api/search/providers': {'candidates':[{'provider':'fixture','provider_id':str(i),'title':f'Provider Track {i}','artist':'Provider Artist','duration_s':45} for i in range(2)],'errors':[]},
            '/api/search': {'tracks': TRACKS, 'albums': [], 'artists': []},
            '/api/downloads': [
                {'id': 709, 'status': 'done', 'track_ref': 'dz:saved', 'nd_song_id': 'fixture-1',
                 'title': 'Saved', 'artist': '', 'album': None, 'star': True},
                {'id': 708, 'status': 'done', 'nd_song_id': 'fixture-2',
                 'title': 'A very long downloaded fixture track with enough text to change the player width',
                 'artist': 'Fixture Artist and a very long credit', 'album': 'A very long album title'},
            ] if query.get('status') == ['done'] else [dict(self.download_item)],
        }
        return (200, routes[path]) if path in routes else (404, {'detail': 'Unexpected fixture route'})


@contextmanager
def backend(fixture=None):
    fixture = fixture or Fixture()

    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *_):
            pass

        def do_GET(self):
            self.handle_request()

        def do_POST(self):
            self.handle_request()

        def do_HEAD(self):
            self.handle_request()

        def handle_request(self):
            parsed = urlsplit(self.path)
            path, query = parsed.path, parse_qs(parsed.query)
            body = None
            if self.command == 'POST':
                try:
                    body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))))
                except (ValueError, TypeError):
                    self.send_error(400)
                    return
            with fixture.lock:
                failed = path in fixture.failures
                delay = fixture.delays.get(path, 0)
            if delay:
                time.sleep(delay)
            headers = {}
            if path == '/fixture/cover.png' and not failed:
                data, status = (fixture.image(query) if hasattr(fixture,"image") else fixture.cover), 200
                headers = {'Content-Type':'image/png'}
            elif path == '/api/listen/stream' and query.get('ref') in ([PREVIEW_TRACK['ref']], [UNAVAILABLE_TRACK['ref']]):
                preview = query['ref'] == [PREVIEW_TRACK['ref']]
                status = 503 if preview else 404
                detail = 'Полная версия пока недоступна. Осталось превью.' if preview else 'Трек не нашёлся ни в одном источнике'
                data = json.dumps({'detail': detail}).encode()
                headers = {'Content-Type': 'application/json'}
            elif (path.startswith('/fixture/audio') or (path == '/api/listen/stream' and fixture.listen_stream)) and self.command in ('GET', 'HEAD') and not failed:
                media = (fixture.audio, 'audio/wav') if path in ('/fixture/audio.wav', '/api/listen/stream') else fixture.extra_audio.get(path)
                data, content_type = media or (b'', 'application/octet-stream')
                status = 200 if data else 404
                headers = {'Content-Type': content_type, 'Accept-Ranges': 'bytes'}
                if path == '/api/listen/stream':
                    headers['X-HomeMusic-Listen-Kind'] = 'library'
                requested_range = self.headers.get('Range')
                if requested_range:
                    try:
                        first, last = requested_range.removeprefix('bytes=').split('-', 1)
                        start = int(first)
                        end = min(int(last) if last else len(data)-1, len(data)-1)
                        if not (0 <= start <= end):
                            raise ValueError()
                        headers['Content-Range'] = f'bytes {start}-{end}/{len(data)}'
                        data, status = data[start:end+1], 206
                    except ValueError:
                        data, status = b'', 416
            else:
                status, value = (503, {'detail': 'Controlled fixture failure'}) if failed else fixture.response(self.command, path, body, query)
                data = json.dumps(value).encode()
                headers['Content-Type'] = 'application/json'
                if path == '/api/auth/login' and status == 200:
                    headers['Set-Cookie'] = 'hm_session=fixture; Path=/'
            with fixture.lock:
                fixture.requests.append({'method': self.command, 'path': path, 'query': query, 'status': status})
            self.send_response(status)
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            if self.command != 'HEAD':
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .1}, daemon=True)
    thread.start()
    try:
        yield fixture, f'http://127.0.0.1:{server.server_port}'
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
