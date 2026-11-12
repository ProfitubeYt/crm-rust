# Chromium with Playwright for page_shots.py. The browsers come with the base
# image; the package that drives them does not, and the stack's network has no
# internet to fetch it from, so it is put in here.
FROM mcr.microsoft.com/playwright/python:v1.49.1-noble
RUN pip install --no-cache-dir --break-system-packages playwright==1.49.1
