#!/bin/bash
# -------------------------------------------------------------------------
# Melawy OS Infrastructure Automation Script
# Сборка раздельных таргетированных артефактов для PKGBUILD в монорепозитории
# Публикация Release через совместимый REST API (GitHub + Forgejo)
# -------------------------------------------------------------------------
set -euo pipefail

# 1. Извлекаем текущую версию из Cargo.toml сервера
VERSION=$(grep -m 1 '^version' flat-scanner-server/Cargo.toml | awk -F'\"' '{print $2}')

if [ -z "$VERSION" ]; then
    echo "❌ Ошибка: Не удалось распарсить версию из flat-scanner-server/Cargo.toml"
    exit 1
fi

echo "📦 Обнаружена общая версия экосистемы: v$VERSION"

# 2. Создаем структуру директорий для артефактов в корне
ARCHIVE_DIR="archive/refs/tags"
mkdir -p "$ARCHIVE_DIR"

SERVER_TAR="$ARCHIVE_DIR/v$VERSION-server.tar.gz"
CLIENT_TAR="$ARCHIVE_DIR/v$VERSION-client.tar.gz"

echo "🧹 Очистка старых локальных архивов..."
rm -f "$SERVER_TAR" "$CLIENT_TAR"

# 2.1. Предварительная очистка артефактов сборки, чтобы они не попали в релизные архивы
echo "🧼 Очистка артефактов сборки (cargo clean / flutter clean)..."
if command -v cargo >/dev/null 2>&1; then
    (cd flat-scanner-server && cargo clean) || echo "⚠️  cargo clean завершился с ошибкой, продолжаю"
else
    echo "⚠️  cargo не найден в PATH, пропускаю cargo clean"
fi

if command -v flutter >/dev/null 2>&1; then
    (cd flat-scanner-client-flutter && flutter clean) || echo "⚠️  flutter clean завершился с ошибкой, продолжаю"
else
    echo "⚠️  flutter не найден в PATH, пропускаю flutter clean"
fi
# Дополнительно удаляем кэши/билды, которые могут остаться после clean
rm -rf flat-scanner-server/target
rm -rf flat-scanner-client-flutter/build flat-scanner-client-flutter/.dart_tool

# 3. Сборка архива сервера (упаковываем ТОЛЬКО содержимое папки flat-scanner-server)
echo "⚙️  Сборка артефакта сервера..."
tar -czf "$SERVER_TAR" -C flat-scanner-server --transform "s?^\.?flat-scanner-server-$VERSION?" .

# 4. Сборка архива клиента (упаковываем ТОЛЬКО содержимое папки flat-scanner-client-flutter)
echo "🎨 Сборка артефакта клиента..."
tar -czf "$CLIENT_TAR" -C flat-scanner-client-flutter --transform "s?^\.?flat-scanner-client-$VERSION?" .

echo "✅ Архивы успешно сгенерированы локально:"
echo "   -> $SERVER_TAR"
echo "   -> $CLIENT_TAR"

# 5. Интеграция с Git-контуром
echo "🏷️  Создание локального тега Git..."
git tag -d "v$VERSION" 2>/dev/null || true
git tag -a "v$VERSION" -m "Release v$VERSION (Split MonoRepo Artifacts)"

echo "🚀 Отправка изменений и тегов на remote..."
git push origin main
git push origin "v$VERSION" --force

# 6. Публикация Release через REST API (GitHub / Forgejo совместимы)
REMOTE_URL=$(git remote get-url origin)
echo "🔗 Remote: $REMOTE_URL"

# Детект API base и owner/repo из remote URL
# Поддерживаемые форматы:
#   https://github.com/OWNER/REPO.git
#   https://git.melawy.ru/OWNER/REPO.git
#   git@github.com:OWNER/REPO.git
if [[ "$REMOTE_URL" =~ github\.com ]]; then
    API_BASE="https://api.github.com"
elif [[ "$REMOTE_URL" =~ git\.melawy\.ru ]]; then
    API_BASE="https://git.melawy.ru"
else
    API_BASE="$REMOTE_URL"
fi

# Извлекаем owner/repo из URL (убираем .git и протокол; для SSH git@host:owner/repo заменяем ':' на '/')
OWNER_REPO=$(echo "$REMOTE_URL" | sed -E 's#^https?://##; s#^git@##; s#:#/#; s#\.git$##')
# Для https URL вида host/owner/repo — оставляем owner/repo (убираем хост)
OWNER_REPO=$(echo "$OWNER_REPO" | sed -E 's#^[^/]+/##')

echo "📡 API: $API_BASE/repos/$OWNER_REPO/releases"

# Токен: приоритет GITHUB_TOKEN, затем FORGEJO_TOKEN, затем gh CLI
TOKEN="${GITHUB_TOKEN:-${FORGEJO_TOKEN:-}}"
if [ -z "$TOKEN" ] && command -v gh >/dev/null 2>&1; then
    TOKEN=$(gh auth token 2>/dev/null || true)
fi

if [ -z "$TOKEN" ]; then
    echo "⚠️  Токен не найден (GITHUB_TOKEN / FORGEJO_TOKEN / gh auth). Пропускаю публикацию Release."
    echo "   Архивы доступны локально в $ARCHIVE_DIR/"
    exit 0
fi

AUTH_HEADER="Authorization: token $TOKEN"

# 6.1. Создаём Release (или получаем существующий по тегу)
RELEASE_JSON=$(curl -sS -X POST \
    -H "$AUTH_HEADER" \
    -H "Content-Type: application/json" \
    -d "{\"tag_name\":\"v$VERSION\",\"name\":\"Release v$VERSION\",\"body\":\"Split MonoRepo Artifacts: server + client\",\"draft\":false,\"prerelease\":false}" \
    "$API_BASE/repos/$OWNER_REPO/releases")

RELEASE_ID=$(echo "$RELEASE_JSON" | jq -r '.id // empty')
if [ -z "$RELEASE_ID" ]; then
    # Возможно релиз уже существует — ищем по тегу
    RELEASE_ID=$(curl -sS -H "$AUTH_HEADER" \
        "$API_BASE/repos/$OWNER_REPO/releases/tags/v$VERSION" | jq -r '.id // empty')
fi

if [ -z "$RELEASE_ID" ]; then
    echo "❌ Ошибка создания Release: $RELEASE_JSON"
    exit 1
fi

echo "✅ Release создан (id=$RELEASE_ID)"

# 6.2. Загружаем артефакты как assets
upload_asset() {
    local FILE_PATH="$1"
    local FILE_NAME="$2"
    local CONTENT_TYPE="application/gzip"

    echo "📤 Загрузка артефакта: $FILE_NAME"
    curl -sS -X POST \
        -H "$AUTH_HEADER" \
        -H "Content-Type: $CONTENT_TYPE" \
        -H "Content-Length: $(stat -c%s "$FILE_PATH")" \
        -H "Content-Transfer-Encoding: binary" \
        -H "Content-Disposition: attachment; filename=\"$FILE_NAME\"" \
        --data-binary @"$FILE_PATH" \
        "$API_BASE/repos/$OWNER_REPO/releases/$RELEASE_ID/assets" > /dev/null
}

upload_asset "$SERVER_TAR" "v$VERSION-server.tar.gz"
upload_asset "$CLIENT_TAR" "v$VERSION-client.tar.gz"

echo "🎉 Релиз v$VERSION успешно опубликован под управлением Core OS Pipeline."
echo "   Assets: v$VERSION-server.tar.gz, v$VERSION-client.tar.gz"