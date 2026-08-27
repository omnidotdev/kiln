# A Flask app served by gunicorn (the provider's Flask start command). Exercises
# the pip install path, copying site-packages into the slim runtime, and the
# gunicorn entrypoint.
from flask import Flask

app = Flask(__name__)


@app.route("/")
def index():
    return "ok\n"
