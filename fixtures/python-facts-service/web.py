"""Flask half of the python-facts fixture: blueprints and request hooks."""
from flask import Flask, Blueprint


def make_web() -> Flask:
    """Assemble a flask app with a blueprint and a request hook."""
    bp = Blueprint("admin", __name__)

    @bp.get("/admin")
    def admin() -> str:
        return "admin"

    @bp.before_request
    def log_request() -> None:
        pass

    app = Flask(__name__)
    app.register_blueprint(bp)
    app.config["DEBUG"] = True
    return app
