#include <rime_api.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include <string>
#include <sys/stat.h>
#include <unistd.h>

namespace {

constexpr const char* kDefaultSharedDataDir =
    "/Library/Input Methods/Squirrel.app/Contents/SharedSupport";
constexpr const char* kDefaultUserDataDirPrefix = "/tmp/inputia-rime-user-";

void ensure_dir(const char* path) {
  if (mkdir(path, 0755) != 0 && errno != EEXIST) {
    std::perror(path);
    std::exit(1);
  }
}

void print_context(RimeApi* rime, RimeSessionId session_id) {
  RIME_STRUCT(RimeContext, context);
  if (!rime->get_context(session_id, &context)) {
    std::puts("context=<unavailable>");
    return;
  }

  std::printf("preedit=%s\n", context.composition.preedit ? context.composition.preedit : "");
  std::printf("cursor=%d selection=%d..%d\n",
              context.composition.cursor_pos,
              context.composition.sel_start,
              context.composition.sel_end);
  std::printf("page=%d page_size=%d candidates=%d last=%d highlighted=%d\n",
              context.menu.page_no,
              context.menu.page_size,
              context.menu.num_candidates,
              context.menu.is_last_page,
              context.menu.highlighted_candidate_index);

  for (int i = 0; i < context.menu.num_candidates; ++i) {
    const auto& candidate = context.menu.candidates[i];
    std::printf("candidate[%d]=%s\t%s\n",
                i,
                candidate.text ? candidate.text : "",
                candidate.comment ? candidate.comment : "");
  }

  rime->free_context(&context);
}

void print_commit(RimeApi* rime, RimeSessionId session_id) {
  RIME_STRUCT(RimeCommit, commit);
  if (rime->get_commit(session_id, &commit)) {
    std::printf("commit=%s\n", commit.text ? commit.text : "");
    rime->free_commit(&commit);
  } else {
    std::puts("commit=<none>");
  }
}

void print_schema(RimeApi* rime, RimeSessionId session_id) {
  char schema_id[128] = {0};
  if (rime->get_current_schema(session_id, schema_id, sizeof(schema_id))) {
    std::printf("schema=%s\n", schema_id);
  }
}

}  // namespace

int main(int argc, char** argv) {
  const char* schema_id = argc > 1 ? argv[1] : "luna_pinyin_simp";
  const char* key_sequence = argc > 2 ? argv[2] : "ni";
  std::vector<std::pair<std::string, bool>> options;
  for (int i = 3; i < argc; ++i) {
    constexpr const char* kOptionPrefix = "--option";
    if (std::strcmp(argv[i], kOptionPrefix) == 0 && i + 1 < argc) {
      std::string option = argv[++i];
      auto separator = option.find('=');
      std::string name = separator == std::string::npos ? option : option.substr(0, separator);
      std::string value = separator == std::string::npos ? "true" : option.substr(separator + 1);
      options.emplace_back(name, value != "false" && value != "0" && value != "off");
    }
  }
  const char* shared_data_dir = std::getenv("INPUTIA_RIME_SHARED_DATA_DIR");
  const char* user_data_dir = std::getenv("INPUTIA_RIME_USER_DATA_DIR");
  if (!shared_data_dir) shared_data_dir = kDefaultSharedDataDir;
  std::string generated_user_data_dir;
  if (!user_data_dir) {
    generated_user_data_dir = std::string(kDefaultUserDataDirPrefix) + std::to_string(getpid());
    user_data_dir = generated_user_data_dir.c_str();
  }

  ensure_dir(user_data_dir);

  RimeApi* rime = rime_get_api();
  RIME_STRUCT(RimeTraits, traits);
  traits.shared_data_dir = shared_data_dir;
  traits.user_data_dir = user_data_dir;
  traits.distribution_name = "Inputia Rime Spike";
  traits.distribution_code_name = "inputia-rime-spike";
  traits.distribution_version = "0.0.1";
  traits.app_name = "rime.inputia-spike";
  traits.min_log_level = 2;
  traits.log_dir = "";

  rime->setup(&traits);
  rime->initialize(&traits);
  if (rime->start_maintenance(False)) {
    rime->join_maintenance_thread();
  }

  RimeSessionId session_id = rime->create_session();
  if (!session_id) {
    std::fputs("failed to create rime session\n", stderr);
    rime->finalize();
    return 1;
  }

  if (!rime->select_schema(session_id, schema_id)) {
    std::fprintf(stderr, "failed to select schema: %s\n", schema_id);
    rime->destroy_session(session_id);
    rime->finalize();
    return 1;
  }

  for (const auto& [name, value] : options) {
    rime->set_option(session_id, name.c_str(), value ? True : False);
    std::printf("option[%s]=%d\n", name.c_str(), rime->get_option(session_id, name.c_str()));
  }

  print_schema(rime, session_id);
  std::printf("keys=%s\n", key_sequence);

  if (!rime->simulate_key_sequence(session_id, key_sequence)) {
    std::fprintf(stderr, "failed to simulate key sequence: %s\n", key_sequence);
    rime->destroy_session(session_id);
    rime->finalize();
    return 1;
  }

  print_context(rime, session_id);

  if (rime->simulate_key_sequence(session_id, "space")) {
    print_commit(rime, session_id);
  }

  rime->destroy_session(session_id);
  rime->finalize();
  return 0;
}
