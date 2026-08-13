"""Celery half of the python-facts fixture: task registration."""
from celery import Celery

celery = Celery("facts")


@celery.task
def send_email(address: str) -> None:
    """Send an email (placeholder)."""
    return None
