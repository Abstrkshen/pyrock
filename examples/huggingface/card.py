from huggingface_hub import ModelCard


def summary():
    card = ModelCard(
        """---
language:
- en
license: mit
library_name: pyrock
---
# pyrock
This program was compiled by pyrock and imports huggingface_hub.
"""
    )
    lang = card.data.language[0]
    title = card.text.strip().split("\n")[0]
    return title + " | " + card.data.license + " | " + lang + " | hub " + card.__module__.split(".")[0]
