"""Local Ollama configuration and request-scoped DSPy execution.

Importing this module never imports an AI client or contacts a server.
"""

from dataclasses import dataclass
from functools import wraps
import ipaddress
import math
import os
from threading import RLock
from urllib.parse import urlsplit


class AIProviderError(RuntimeError):
    """Actionable error safe to display without exposing request contents."""


@dataclass(frozen=True)
class ProviderConfig:
    provider: str
    model: str
    base_url: str
    timeout_seconds: float

    @classmethod
    def from_env(cls):
        provider = os.environ.get("ARCHGEN_AI_PROVIDER", "ollama").strip().lower()
        if provider != "ollama":
            raise AIProviderError("Unsupported ARCHGEN_AI_PROVIDER. This version supports only 'ollama'.")
        model = os.environ.get("ARCHGEN_AI_MODEL", "llama3.2:3b").strip()
        if not model or any(char.isspace() for char in model):
            raise AIProviderError("Set ARCHGEN_AI_MODEL to an installed Ollama model name, e.g. llama3.2:3b.")
        base_url = os.environ.get("ARCHGEN_OLLAMA_URL", "http://localhost:11434").strip().rstrip("/")
        try:
            url = urlsplit(base_url)
            host = url.hostname
            local = host == "localhost" or (host is not None and ipaddress.ip_address(host).is_loopback)
            valid = (
                local and url.scheme in ("http", "https") and not url.username
                and not url.password and not url.path and not url.query and not url.fragment
                and (url.port is None or url.port > 0)
            )
        except ValueError:
            valid = False
        if not valid:
            raise AIProviderError("ARCHGEN_OLLAMA_URL must be a local loopback HTTP(S) URL, e.g. http://localhost:11434.")
        try:
            timeout = float(os.environ.get("ARCHGEN_AI_TIMEOUT_SECONDS", "120"))
            if not math.isfinite(timeout) or timeout <= 0:
                raise ValueError
        except ValueError:
            raise AIProviderError("ARCHGEN_AI_TIMEOUT_SECONDS must be a finite number greater than zero.") from None
        return cls(provider, model, base_url, timeout)


def _request_error(error, config):
    # Do not forward provider response bodies: they can contain submitted content.
    status = getattr(error, "status_code", None)
    name = type(error).__name__.lower()
    if status == 404:
        return AIProviderError(
            f"Ollama model '{config.model}' or endpoint was not found. Check ARCHGEN_OLLAMA_URL "
            "and install the exact ARCHGEN_AI_MODEL with 'ollama pull <model>'. No other model was selected."
        )
    if isinstance(error, TimeoutError) or "timeout" in name:
        return AIProviderError(
            f"Ollama request timed out (configured timeout: {config.timeout_seconds:g}s). "
            "Check the local server or increase ARCHGEN_AI_TIMEOUT_SECONDS."
        )
    if isinstance(error, ConnectionError) or "connect" in name:
        return AIProviderError("Cannot connect to local Ollama. Start it with 'ollama serve' and check ARCHGEN_OLLAMA_URL.")
    return AIProviderError(
        f"Local AI request failed ({type(error).__name__}). Check Ollama, ARCHGEN_AI_MODEL "
        "and the model's ability to produce the requested output. No provider or model fallback was attempted."
    )


# Existing DSPy modules/compiled graph are shared. Serialize entire jobs to avoid
# mutable predictor/history races. The context override is scoped to each job;
# never use dspy.configure() from arbitrary Rust spawn_blocking worker threads.
_generation_lock = RLock()


def with_local_provider(function):
    @wraps(function)
    def wrapped(*args, **kwargs):
        config = ProviderConfig.from_env()
        with _generation_lock:
            import dspy

            try:
                lm = dspy.LM(
                    f"ollama_chat/{config.model}", api_base=config.base_url,
                    temperature=0.7, max_tokens=4096, cache=False,
                    timeout=config.timeout_seconds, num_retries=0,
                )
                with dspy.context(lm=lm):
                    return function(*args, **kwargs)
            except AIProviderError:
                raise
            except Exception as error:
                raise _request_error(error, config) from None
    return wrapped


def health_check():
    """Check configured model metadata, without generating text or pulling models.

    A successful result proves availability, not successful inference or adequate
    model quality. No application content is sent by this explicit diagnostic.
    """
    config = ProviderConfig.from_env()
    import ollama

    try:
        client = ollama.Client(host=config.base_url, timeout=config.timeout_seconds)
        client.show(config.model)
    except Exception as error:
        raise _request_error(error, config) from None
    return {"status": "ready", "provider": config.provider, "model": config.model, "base_url": config.base_url}
