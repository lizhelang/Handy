#include <rime_api.h>
#include <dlfcn.h>
#include <mach-o/dyld.h>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <stdexcept>
#include <string>

namespace fs = std::filesystem;

static void require(bool value, const std::string& reason) {
  if (!value) throw std::runtime_error(reason);
}

static void verify_images(const fs::path& executable) {
  for (uint32_t i = 0; i < _dyld_image_count(); ++i) {
    const std::string path = _dyld_get_image_name(i);
    require(path.rfind("/usr/lib/", 0) == 0 || path.rfind("/System/Library/", 0) == 0 ||
                fs::equivalent(path, executable),
            "unexpected external image: " + path);
  }
  Dl_info info{};
  require(dladdr(reinterpret_cast<void*>(&rime_get_api), &info) != 0 &&
              fs::equivalent(info.dli_fname, executable),
          "rime_get_api did not originate in the probe executable");
}

static void check_input(RimeApi* api, const char* schema, const char* keys) {
  const auto session = api->create_session();
  require(session != 0 && api->select_schema(session, schema), "schema selection failed");
  api->set_option(session, "ascii_mode", False);
  for (const char* key = keys; *key; ++key) {
    require(api->process_key(session, *key, 0), "Rime did not consume fixture key");
  }
  RIME_STRUCT(RimeContext, context);
  require(api->get_context(session, &context), "no Rime context");
  int selected = -1;
  for (int i = 0; i < context.menu.num_candidates; ++i) {
    const auto& candidate = context.menu.candidates[i];
    if (candidate.text && std::string(candidate.text) == "你好") {
      require(candidate.comment && std::string(candidate.comment) == "static-lua-executed",
              "Lua filter did not execute");
      selected = i;
      break;
    }
  }
  api->free_context(&context);
  require(selected >= 0, "expected synthetic candidate missing");
  require(api->select_candidate(session, selected), "candidate selection failed");
  RIME_STRUCT(RimeCommit, commit);
  require(api->get_commit(session, &commit), "no Rime commit");
  require(commit.text && std::string(commit.text) == "你好", "wrong committed text");
  api->free_commit(&commit);
  api->destroy_session(session);
  std::cout << "schema=" << schema << " keys=" << keys
            << " candidate=true commit=true lua_filter=true\n";
}

int main(int argc, char** argv) {
  try {
    require(argc == 3, "usage: static-rime-probe FIXTURE_DIR NEW_RUN_DIR");
    const fs::path fixture = fs::canonical(argv[1]);
    const fs::path run = fs::canonical(argv[2]);
    const auto user = run / "user";
    require(!fs::exists(user), "probe refuses to reuse any user directory");
    fs::create_directory(user);
    fs::copy_file(fixture / "rime.lua", user / "rime.lua");
    const std::string shared_path = fixture.string(), user_path = user.string();
    RIME_STRUCT(RimeTraits, traits);
    traits.shared_data_dir = shared_path.c_str();
    traits.user_data_dir = user_path.c_str();
    traits.distribution_name = "Inputia static synthetic probe";
    traits.distribution_code_name = "inputia-static-probe";
    traits.distribution_version = "1";
    traits.app_name = "rime.inputia-static-probe";
    traits.log_dir = "";
    traits.min_log_level = 2;
    auto* api = rime_get_api();
    require(api != nullptr, "rime_get_api returned null");
    api->setup(&traits);
    api->initialize(&traits);
    api->deployer_initialize(&traits);
    for (const char* module : {"lua", "octagram", "grammar", "predict"}) {
      require(api->find_module(module) != nullptr, std::string("module absent: ") + module);
      std::cout << "module=" << module << " registered=true\n";
    }
    for (const char* schema : {"static_pinyin", "static_double"}) {
      const auto path = fixture / (std::string(schema) + ".schema.yaml");
      require(api->deploy_schema(path.c_str()), "schema deployment failed");
    }
    verify_images(fs::canonical(argv[0]));
    check_input(api, "static_pinyin", "nihao");
    check_input(api, "static_double", "nihc");
    std::cout << "rime_version=" << api->get_version()
              << " external_dylib_fallback=false synthetic_user_dir=" << user << '\n';
    api->finalize();
    verify_images(fs::canonical(argv[0]));
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "static_rime_probe_failed=" << error.what() << '\n';
    return 1;
  }
}
