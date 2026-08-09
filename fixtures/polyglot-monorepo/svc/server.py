"""HTTP entrypoint for the payment service."""
from payments import handle_payment, list_payments
from http.server import BaseHTTPRequestHandler

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        handle_payment(100)

    def do_GET(self):
        list_payments()

if __name__ == "__main__":
    from http.server import HTTPServer
    HTTPServer(("localhost", 8080), Handler).serve_forever()
