#!/usr/bin/env bash

# Exit immediately if a command exits with a non-zero status
set -e

# Setup terminal colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0;37m' # No Color

API_URL="http://127.0.0.1:3000/api/v1"

# Generate unique run identifiers to prevent database constraint conflicts
RUN_ID=$((1000 + RANDOM % 9000))
USERNAME_STD="user_${RUN_ID}"
USERNAME_ATTACKER="attacker_${RUN_ID}"
USERNAME_INTERN="intern_${RUN_ID}"
POST_TITLE="Asynchronous Programming in Rust ${RUN_ID}"

echo -e "${BLUE}====================================================${NC}"
echo -e "${BLUE}          STARTING SYSTEM LIFE-CYCLE TESTS          ${NC}"
echo -e "${BLUE}          Run ID: ${RUN_ID}                          ${NC}"
echo -e "${BLUE}====================================================${NC}"

# Verification utilities
if ! command -v jq &> /dev/null; then
    echo -e "${RED}[ERROR] 'jq' utility is missing. Install using your system package manager.${NC}"
    exit 1
fi

# Verify server availability before running tests
echo -e "${BLUE}[INFO] Pinging API server at ${API_URL}...${NC}"
if ! curl -s --connect-timeout 2 "${API_URL}/posts" > /dev/null; then
    echo -e "${RED}[ERROR] API server is offline or unreachable at ${API_URL}${NC}"
    exit 1
fi

# Core HTTP request helper with strict assertions
call_api() {
    local method=$1
    local path=$2
    local expected_status=$3
    local payload=$4
    local token=$5

    local headers=()
    headers+=("-H" "Content-Type: application/json")
    if [ -n "$token" ]; then
        headers+=("-H" "Authorization: Bearer $token")
    fi

    local response_file
    response_file=$(mktemp)
    local http_status

    if [ -n "$payload" ]; then
        http_status=$(curl -s -w "%{http_code}" -o "$response_file" -X "$method" "${API_URL}${path}" "${headers[@]}" -d "$payload")
    else
        http_status=$(curl -s -w "%{http_code}" -o "$response_file" -X "$method" "${API_URL}${path}" "${headers[@]}")
    fi

    if [ "$http_status" -ne "$expected_status" ]; then
        echo -e "${RED}[FAIL] ${method} ${path} returned ${http_status}, expected ${expected_status}${NC}" >&2
        echo -e "${RED}Response payload: $(cat "$response_file")${NC}" >&2
        rm -f "$response_file"
        exit 1
    else
        echo -e "${GREEN}[PASS] ${method} ${path} (${http_status})${NC}" >&2
        cat "$response_file"
        rm -f "$response_file"
    fi
}

# ==============================================================================
# PHASE 1: ADMIN LOGIN & SYSTEM SETUP
# ==============================================================================
echo -e "\n${YELLOW}--- PHASE 1: ADMIN SEED CHECK & INGESTION ---${NC}"

# Admin credentials: use env when set (matches ADMIN_* in .env), fallback to dev default.
ADMIN_USERNAME="${ADMIN_USERNAME:-admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-Admin@123456}"
# 1. Admin Login
ADMIN_RESP=$(call_api "POST" "/auth/login" 200 "{\"username\": \"${ADMIN_USERNAME}\", \"password\": \"${ADMIN_PASSWORD}\"}")
ADMIN_TOKEN=$(echo "$ADMIN_RESP" | jq -r '.token')

# 2. Create Tag: Rust
TAG_RUST_RESP=$(call_api "POST" "/tags" 200 "{\"name\": \"Rust-${RUN_ID}\"}" "$ADMIN_TOKEN")
TAG_RUST_ID=$(echo "$TAG_RUST_RESP" | jq -r '.id')
TAG_RUST_SLUG=$(echo "$TAG_RUST_RESP" | jq -r '.slug')

# 3. Create Tag: Async
TAG_ASYNC_RESP=$(call_api "POST" "/tags" 200 "{\"name\": \"Async-${RUN_ID}\"}" "$ADMIN_TOKEN")
TAG_ASYNC_ID=$(echo "$TAG_ASYNC_RESP" | jq -r '.id')

# ==============================================================================
# PHASE 2: AUTHENTICATION LIFECYCLE (REGISTER & LOGIN)
# ==============================================================================
echo -e "\n${YELLOW}--- PHASE 2: USER REGISTRATION & AUTHENTICATION ---${NC}"

# 1. Register Standard User
REGISTER_RESP=$(call_api "POST" "/auth/register" 200 "{\"username\": \"${USERNAME_STD}\", \"email\": \"${USERNAME_STD}@tokio.rs\", \"password\": \"SecurePassword123\"}")
USER_TOKEN=$(echo "$REGISTER_RESP" | jq -r '.token')

# 2. Get standard user profile
call_api "GET" "/users/me" 200 "" "$USER_TOKEN" > /dev/null

# ==============================================================================
# PHASE 3: POSTS LIFECYCLE
# ==============================================================================
echo -e "\n${YELLOW}--- PHASE 3: POSTS LIFECYCLE & SEARCH INDEXING ---${NC}"

# 1. Create Article (By Standard User)
CREATE_POST_PAYLOAD=$(cat <<EOF
{
  "title": "${POST_TITLE}",
  "content": "A high-performance deep dive into Tokio system runtime and safe futures.",
  "excerpt": "High concurrency paradigms in Rust.",
  "published": true,
  "tag_ids": ["${TAG_RUST_ID}"]
}
EOF
)
POST_RESP=$(call_api "POST" "/posts" 200 "$CREATE_POST_PAYLOAD" "$USER_TOKEN")
POST_ID=$(echo "$POST_RESP" | jq -r '.id')
POST_SLUG=$(echo "$POST_RESP" | jq -r '.slug')

# 2. Public Query List (Should find the newly published post)
call_api "GET" "/posts" 200 "" > /dev/null

# 3. Search filter check
call_api "GET" "/posts?search=Tokio" 200 "" > /dev/null

# 4. Tag query filter check
call_api "GET" "/posts?tag=${TAG_RUST_SLUG}" 200 "" > /dev/null

# 5. Fetch Single Post by Slug
call_api "GET" "/posts/${POST_SLUG}" 200 "" > /dev/null

# ==============================================================================
# PHASE 4: SECURITY CONTROLS (RESOURCE ISOLATION)
# ==============================================================================
echo -e "\n${YELLOW}--- PHASE 4: SECURITY VERIFICATION (RBAC & BOUNDARY ISOLATION) ---${NC}"

# 1. Register a separate attacker account
ATTACK_REG_RESP=$(call_api "POST" "/auth/register" 200 "{\"username\": \"${USERNAME_ATTACKER}\", \"email\": \"${USERNAME_ATTACKER}@unsafe.rs\", \"password\": \"HackerInjected123\"}")
ATTACKER_TOKEN=$(echo "$ATTACK_REG_RESP" | jq -r '.token')

# 2. Attempt unauthorized modification (Expect 403 Forbidden)
call_api "PUT" "/posts/${POST_ID}" 403 '{"title": "Hacked Title"}' "$ATTACKER_TOKEN" > /dev/null

# 3. Attempt unauthorized deletion (Expect 403 Forbidden)
call_api "DELETE" "/posts/${POST_ID}" 403 "" "$ATTACKER_TOKEN" > /dev/null

# ==============================================================================
# PHASE 5: NESTED COMMENT TREE (MATERIALIZED PATH)
# ==============================================================================
echo -e "\n${YELLOW}--- PHASE 5: NESTED HIERARCHICAL COMMENT TREE ---${NC}"

# 1. Post Root Comment
COMMENT_ROOT_RESP=$(call_api "POST" "/posts/${POST_ID}/comments" 200 '{"content": "Outstanding technical article!"}' "$ATTACKER_TOKEN")
COMMENT_ROOT_ID=$(echo "$COMMENT_ROOT_RESP" | jq -r '.id')

# 2. Reply to Root Comment (Level 1 Nesting)
COMMENT_LEVEL1_RESP=$(call_api "POST" "/posts/${POST_ID}/comments" 200 "{\"content\": \"Thank you, glad you liked it!\", \"parent_id\": \"${COMMENT_ROOT_ID}\"}" "$USER_TOKEN")
COMMENT_LEVEL1_ID=$(echo "$COMMENT_LEVEL1_RESP" | jq -r '.id')

# 3. Reply to Reply (Level 2 Nesting)
call_api "POST" "/posts/${POST_ID}/comments" 200 "{\"content\": \"Indeed, safe concurrency is great!\", \"parent_id\": \"${COMMENT_LEVEL1_ID}\"}" "$ATTACKER_TOKEN" > /dev/null

# 4. Fetch Nested Comments Tree (Verifies correct structural ordering)
TREE_RESP=$(call_api "GET" "/posts/${POST_ID}/comments" 200 "")

# Assert comment hierarchy counts using jq
CHILDREN_COUNT=$(echo "$TREE_RESP" | jq '.[0].children | length')
if [ "$CHILDREN_COUNT" -ne 1 ]; then
    echo -e "${RED}[FAIL] Tree nesting structures were built incorrectly! Expected 1 child node under root, found ${CHILDREN_COUNT}${NC}"
    exit 1
fi
echo -e "${GREEN}[PASS] Validated nested child structural layout inside JSON response${NC}"

# 5. Soft-Delete Parent Comment (Verifies tree structure is preserved)
call_api "DELETE" "/posts/${POST_ID}/comments/${COMMENT_ROOT_ID}" 200 "" "$ATTACKER_TOKEN" > /dev/null

# Verify that the parent node has been replaced with '[deleted]' but remains in the tree
DELETED_TREE_CHECK=$(call_api "GET" "/posts/${POST_ID}/comments" 200 "")
ROOT_CONTENT=$(echo "$DELETED_TREE_CHECK" | jq -r '.[0].content')
IS_DELETED_STATE=$(echo "$DELETED_TREE_CHECK" | jq -r '.[0].is_deleted')

if [ "$ROOT_CONTENT" != "[deleted]" ] || [ "$IS_DELETED_STATE" != "true" ]; then
    echo -e "${RED}[FAIL] Soft-delete assertion failed! Content: ${ROOT_CONTENT}, is_deleted: ${IS_DELETED_STATE}${NC}"
    exit 1
fi
echo -e "${GREEN}[PASS] Soft-delete preserved comments tree structure successfully${NC}"

# ==============================================================================
# PHASE 6: ADMINISTRATIVE MODERATION CONTROLS
# ==============================================================================
echo -e "\n${YELLOW}--- PHASE 6: ADMINISTRATIVE AUDIT & ROLE MANAGEMENT ---${NC}"

# 1. Register temporary account to audit
TEMP_USER_RESP=$(call_api "POST" "/auth/register" 200 "{\"username\": \"${USERNAME_INTERN}\", \"email\": \"${USERNAME_INTERN}@corp.com\", \"password\": \"Temp12345\"}")
TEMP_USER_ID=$(echo "$TEMP_USER_RESP" | jq -r '.user.id')

# 2. Get list of system users (Admin Only)
call_api "GET" "/admin/users" 200 "" "$ADMIN_TOKEN" > /dev/null

# 3. Elevate user to Moderator role (Admin Only)
call_api "PUT" "/admin/users/${TEMP_USER_ID}/role" 200 '{"role": "moderator"}' "$ADMIN_TOKEN" > /dev/null

# Verify role elevation by logging in as the newly promoted user
INTERN_TOKEN=$(call_api "POST" "/auth/login" 200 "{\"username\": \"${USERNAME_INTERN}\", \"password\": \"Temp12345\"}" | jq -r '.token')
INTERN_PROFILE=$(call_api "GET" "/users/me" 200 "" "$INTERN_TOKEN")
ASSIGNED_ROLE=$(echo "$INTERN_PROFILE" | jq -r '.role')

if [ "$ASSIGNED_ROLE" != "moderator" ]; then
    echo -e "${RED}[FAIL] Role elevation validation failed. Role is '${ASSIGNED_ROLE}', expected 'moderator'${NC}"
    exit 1
fi
echo -e "${GREEN}[PASS] Elevated role assignment validated successfully${NC}"

# 4. Deactivate the elevated user account (Admin Only)
call_api "POST" "/admin/users/${TEMP_USER_ID}/deactivate" 200 "" "$ADMIN_TOKEN" > /dev/null

# Verify account lock (Login should fail with 401 Unauthorized)
call_api "POST" "/auth/login" 401 "{\"username\": \"${USERNAME_INTERN}\", \"password\": \"Temp12345\"}" > /dev/null
echo -e "${GREEN}[PASS] Account deactivation lock verified successfully${NC}"

# ==============================================================================
# SUMMARY
# ==============================================================================
echo -e "\n${GREEN}====================================================${NC}"
echo -e "${GREEN}   ALL SYSTEM INTEGRATION CHECKS COMPLETED: SUCCESS  ${NC}"
echo -e "${GREEN}====================================================${NC}"
exit 0
