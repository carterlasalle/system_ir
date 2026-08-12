"""cli-service: demo CLI with two subcommands, flags, and store writes."""

import argparse
import sqlite3


def build_parser():
    """Construct the argparse CLI surface."""
    parser = argparse.ArgumentParser(prog="cli-service")
    parser.add_argument("--verbose", action="store_true")
    sub = parser.add_subparsers(dest="command")
    serve = sub.add_parser("serve", help="serve requests")
    serve.add_argument("--port", type=int, default=8080)
    serve.add_argument("--paging", action="store_true")
    deploy = sub.add_parser("deploy", help="deploy the build")
    deploy.add_argument("--env", choices=["dev", "prod"], default="dev")
    return parser


def serve(args):
    """Serve requests; records each invocation."""
    conn = sqlite3.connect("cli.db")
    conn.execute("INSERT INTO events (kind) VALUES (?)", ("serve",))
    conn.commit()
    conn.close()
    return args.port


def deploy(args):
    """Deploy the build; persists the deployment."""
    session = Session()
    session.add(Deployment(env=args.env))
    session.commit()
    return args.env


def main():
    """Entrypoint: parse and dispatch."""
    args = build_parser().parse_args()
    if args.command == "serve":
        serve(args)
    else:
        deploy(args)


if __name__ == "__main__":
    main()
