# syntax=docker/dockerfile:1
# ── Build stage ──────────────────────────────────────────────────────────────
FROM node:20-alpine AS builder
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
# Production build. NOTE: `ng build --configuration production` inlines Google
# Fonts, so the build host needs outbound internet to fonts.googleapis.com.
RUN npm run build

# ── Runtime stage (static) ───────────────────────────────────────────────────
FROM nginx:alpine AS runtime
# @angular/build:application emits the browser bundle under dist/frontend/browser.
COPY --from=builder /app/dist/frontend/browser /usr/share/nginx/html
COPY nginx-spa.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
