from .direct import get_package as get_direct_package
from .google_omaha import get_package as get_google_omaha_package
from .microsoft_edge import get_package as get_microsoft_edge_package


PROVIDERS = {
    "direct": get_direct_package,
    "google_omaha": get_google_omaha_package,
    "microsoft_edge": get_microsoft_edge_package,
}


def get_package(provider_config):
    provider_type = provider_config.get("type")
    if provider_type not in PROVIDERS:
        available = ", ".join(sorted(PROVIDERS))
        raise KeyError(f"Unknown provider '{provider_type}'. Available providers: {available}")
    return PROVIDERS[provider_type](provider_config)
