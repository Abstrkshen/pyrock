# pyrock run main.py
import server
import client

port = server.start(0)
print("listening", port)
print(client.get(port, "/"))
print(client.get(port, "/pyrock"))
