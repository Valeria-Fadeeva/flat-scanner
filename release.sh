#!/bin/bash
# -------------------------------------------------------------------------
# Melawy OS Infrastructure Automation Script
# Сборка раздельных таргетированных артефактов для PKGBUILD в монорепозитории
# -------------------------------------------------------------------------

# 1. Извлекаем текущую версию из Cargo.toml сервера
VERSION=$(grep -m 1 '^version' flat-scanner-server/Cargo.toml | awk -F'\"' '{print $2}')

if [ -z "$VERSION" ]; then
    echo "❌ Ошибка: Не удалось распарсить версию из flat-scanner-server/Cargo.toml"
    exit 1
fi

echo "📦 Обнаружена общая версия экосистемы: v$VERSION"

# 2. Создаем структуру директорий для артефактов в корне (имитируем зеркало путей)
ARCHIVE_DIR="archive/refs/tags"
mkdir -p "$ARCHIVE_DIR"

# Имена финальных архивных файлов
SERVER_TAR="$ARCHIVE_DIR/v$VERSION-server.tar.gz"
CLIENT_TAR="$ARCHIVE_DIR/v$VERSION-client.tar.gz"

echo "🧹 Очистка старых локальных архивов..."
rm -f "$SERVER_TAR" "$CLIENT_TAR"

# 3. Сборка архива сервера (упаковываем ТОЛЬКО содержимое папки flat-scanner-server)
# Флаг -C переключает контекст tar внутрь папки, чтобы в корне архива не было лишней вложенности
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
# Удаляем тег, если он уже был ошибочно поставлен локально
git tag -d "v$VERSION" 2>/dev/null
git tag -a "v$VERSION" -m "Release v$VERSION (Split MonoRepo Artifacts)"

echo "🚀 Отправка изменений и тегов на git.melawy.ru..."
git push origin main
git push origin "v$VERSION" --force

echo "🎉 Релиз v$VERSION успешно опубликован под управлением Core OS Pipeline."
