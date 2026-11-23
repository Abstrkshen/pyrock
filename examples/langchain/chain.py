from langchain_core.prompts import PromptTemplate
from langchain_core.runnables import RunnableLambda


def reply(name):
    prompt = PromptTemplate.from_template("Hello {name}, this is pyrock.")
    text = prompt.format(name=name)
    upper = RunnableLambda(lambda s: s.upper()) | RunnableLambda(lambda s: s + " [langchain]")
    return upper.invoke(text)
