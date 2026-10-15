import base64
import hashlib
import secrets
import urllib.request
import urllib.parse
import urllib.error
import json
import sys
import os
import ssl

class ConditionalRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if "localhost:8080/callback" in newurl:
            return None
        return super().redirect_request(req, fp, code, msg, headers, newurl)

# An API serving HTTPS from a private CA (as the Helm chart's does) is verified against
# STORMCHASER_CA_CERT, as the CLI does.
ssl_context = ssl.create_default_context(cafile=os.environ.get("STORMCHASER_CA_CERT") or None)
opener = urllib.request.build_opener(
    ConditionalRedirectHandler(), urllib.request.HTTPSHandler(context=ssl_context)
)
urllib.request.install_opener(opener)

def get_token():
    host_dex = os.environ.get("HOST_DEX", "127.0.0.1")
    host_api = os.environ.get("HOST_API", "127.0.0.1")
    port_dex = os.environ.get("PORT_DEX", "5556")
    port_api = os.environ.get("PORT_API", "3000")
    # API_URL (for example https://localhost:13000) wins over HOST_API and PORT_API
    api_url = os.environ.get("API_URL") or f"http://{host_api}:{port_api}"

    def exchange(final_url):
        query = urllib.parse.urlparse(final_url).query
        code = urllib.parse.parse_qs(query).get('code', [''])[0]

        api_data = json.dumps({
            'sso_token': code,
            'callback_url': 'http://localhost:8080/callback',
            'code_verifier': code_verifier
        }).encode('utf-8')
        api_req = urllib.request.Request(f"{api_url}/api/v1/auth/exchange", data=api_data, method='POST')
        api_req.add_header('Content-Type', 'application/json')
        api_resp = urllib.request.urlopen(api_req)
        print(json.loads(api_resp.read())['access_token'])

    def get_password(username):
        try:
            with open("deploy/dex/credentials.generated", "r") as f:
                for line in f:
                    if line.startswith(username + ":"):
                        return line.split(":", 1)[1].strip()
        except Exception:
            pass
        return "password"

    # The API redeems a code only with the PKCE verifier its login was started with
    code_verifier = secrets.token_urlsafe(48)
    code_challenge = base64.urlsafe_b64encode(
        hashlib.sha256(code_verifier.encode('ascii')).digest()
    ).decode('ascii').rstrip('=')

    auth_url = f"http://{host_dex}:{port_dex}/dex/auth?client_id=stormchaser-cli&redirect_uri=http://localhost:8080/callback&response_type=code&scope=openid+profile+email&code_challenge={code_challenge}&code_challenge_method=S256"
    req1 = urllib.request.Request(auth_url)
    try:
        resp1 = urllib.request.urlopen(req1)
        html = resp1.read().decode('utf-8')
        action_start = html.find('action="') + 8
        action_end = html.find('"', action_start)
        action = html[action_start:action_end]
        login_url = f"http://{host_dex}:{port_dex}" + action.replace('&amp;', '&')

        password = get_password('stormchaser-admin@paninfracon.net')
        data = urllib.parse.urlencode({'login': 'stormchaser-admin@paninfracon.net', 'password': password}).encode('ascii')
        req2 = urllib.request.Request(login_url, data=data, method='POST')

        try:
            resp2 = urllib.request.urlopen(req2)
            html2 = resp2.read().decode('utf-8')

            if "Grant Access" in html2:
                req_start = html2.find('name="req" value="') + 18
                req_end = html2.find('"', req_start)
                req_val = html2[req_start:req_end]

                app_data = urllib.parse.urlencode({'approval': 'approve', 'req': req_val}).encode('ascii')
                app_req = urllib.request.Request(resp2.geturl(), data=app_data, method='POST')
                try:
                    resp3 = urllib.request.urlopen(app_req)
                    final_url = resp3.geturl()
                except urllib.error.HTTPError as e:
                    if e.code in [302, 303] and "localhost:8080/callback" in e.headers.get('Location', ''):
                        final_url = e.headers['Location']
                    else:
                        raise
            else:
                final_url = resp2.geturl()

            exchange(final_url)

        except urllib.error.HTTPError as e:
            if e.code in [302, 303] and "localhost:8080/callback" in e.headers.get('Location', ''):
                exchange(e.headers['Location'])
            else:
                print("HTTP Error:", e.code, e.headers.get('Location', ''), file=sys.stderr)
                sys.exit(1)

    except Exception as e:
        import traceback
        traceback.print_exc()
        sys.exit(1)

if __name__ == '__main__':
    get_token()
