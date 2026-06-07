var V=Object.defineProperty;var Z=(e,n,t)=>n in e?V(e,n,{enumerable:!0,configurable:!0,writable:!0,value:t}):e[n]=t;var N=(e,n,t)=>Z(e,typeof n!="symbol"?n+"":n,t);import{i as K,d as L,s as Y,c as Q,R as X,t as ee,g as te,F as ne,n as S,a as ae}from"./decorators-Bnaor6Ku.js";const re={refact_impact_dates:{data:{daily:{"2023-12-15":{completions:14,human:52,langs:[".rs",".py"],refact:203,refact_impact:.7960784435272217,total:255},"2023-12-18":{completions:16,human:83,langs:[".py"],refact:245,refact_impact:.7469512224197388,total:328},"2023-12-19":{completions:6,human:4,langs:[".cpp"],refact:103,refact_impact:.9626168012619019,total:107},"2023-12-20":{completions:46,human:857,langs:[".py"],refact:693,refact_impact:.4470967650413513,total:1550},"2023-12-21":{completions:92,human:1157,langs:[".py"],refact:3103,refact_impact:.7284037470817566,total:4260},"2023-12-22":{completions:59,human:-38,langs:[".py"],refact:2005,refact_impact:1.0193188190460205,total:1967},"2023-12-27":{completions:13,human:28,langs:[".py"],refact:409,refact_impact:.9359267950057983,total:437},"2023-12-29":{completions:2,human:2,langs:[".py"],refact:71,refact_impact:.9726027250289917,total:73},"2024-01-04":{completions:12,human:1772,langs:[".rs"],refact:303,refact_impact:.14602409303188324,total:2075},"2024-01-09":{completions:4,human:33,langs:[".py"],refact:166,refact_impact:.8341708779335022,total:199},"2024-01-24":{completions:10,human:808,langs:[".rs"],refact:410,refact_impact:.3366174101829529,total:1218},"2024-01-25":{completions:76,human:7993,langs:[".rs"],refact:2772,refact_impact:.25750115513801575,total:10765},"2024-01-26":{completions:21,human:1931,langs:[".rs"],refact:557,refact_impact:.22387459874153137,total:2488},"2024-01-29":{completions:21,human:2574,langs:[".rs"],refact:655,refact_impact:.20284917950630188,total:3229},"2024-01-30":{completions:29,human:1849,langs:[".rs"],refact:1310,refact_impact:.41468819975852966,total:3159},"2024-01-31":{completions:31,human:3452,langs:[".rs",".txt"],refact:1114,refact_impact:.24397721886634827,total:4566},"2024-02-01":{completions:57,human:8806,langs:[".rs",".txt"],refact:2465,refact_impact:.21870286762714386,total:11271},"2024-02-02":{completions:11,human:5869,langs:[".rs",".txt",".py"],refact:307,refact_impact:.04970854893326759,total:6176},"2024-02-05":{completions:5,human:1976,langs:[".rs",".txt"],refact:233,refact_impact:.10547759383916855,total:2209}},weekly:{"2023-12-15":{completions:14,human:52,langs:[".py",".rs"],refact:203,refact_impact:.7960784435272217,total:255},"2023-12-22":{completions:219,human:2063,langs:[".py",".cpp"],refact:6149,refact_impact:.7487822771072388,total:8212},"2023-12-27":{completions:15,human:30,langs:[".py"],refact:480,refact_impact:.9411764740943909,total:510},"2024-01-04":{completions:12,human:1772,langs:[".rs"],refact:303,refact_impact:.14602409303188324,total:2075},"2024-01-09":{completions:4,human:33,langs:[".py"],refact:166,refact_impact:.8341708779335022,total:199},"2024-01-24":{completions:107,human:10732,langs:[".rs"],refact:3739,refact_impact:.2583788335323334,total:14471},"2024-02-02":{completions:149,human:22550,langs:[".rs",".py",".txt"],refact:5851,refact_impact:.20601387321949005,total:28401},"2024-02-05":{completions:5,human:1976,langs:[".rs",".txt"],refact:233,refact_impact:.10547759383916855,total:2209}}}},table_refact_impact:{columns:["Language","Refact","Human","Total (characters)","Refact Impact","Completions"],data:[{completions:276,human:31996,lang:".rs",refact:10092,refact_impact:.23978331685066223,total:42088},{completions:243,human:7110,lang:".py",refact:6929,refact_impact:.49355366826057434,total:14039},{completions:6,human:4,lang:".cpp",refact:103,refact_impact:.9626168012619019,total:107},{completions:0,human:98,lang:".txt",refact:0,refact_impact:0,total:98}],title:"Refact's impact by language"}};JSON.stringify(re);const Je={uncommited_changes_warning:"You have uncommitted changes:\n```\nIn project refact-lsp: A tests/emergency_frog_situation/.refact/integrations.d/github.yaml, M tests/emergency_frog_situation/frog.py, M tests/emergency_frog_situation/jump_to_conclusions.py, ...\n```\n⚠️ You might have a problem rolling back agent's changes.",new_chat_suggestion:!1,links:[{link_text:"Save and return",link_action:"patch-all",link_goto:"SETTINGS:/path/to/config/file.yaml",link_tooltip:""},{link_text:"Save and Continue",link_action:"patch-all",link_goto:"NEWCHAT",link_tooltip:""},{link_text:"Can you fix it?",link_action:"follow-up",link_tooltip:"a nice tool tip message"},{link_action:"post-chat",link_text:"Stop recommending integrations",link_tooltip:"",link_payload:{chat_meta:{chat_id:"",chat_remote:!1,chat_mode:"CONFIGURE",current_config_file:"/Users/kot/code_aprojects/demotest/.refact/integrations.d/github.yaml"},messages:[{role:"user",content:"Make recommended_integrations an empty list, follow the system prompt."}]}},{link_action:"commit",link_text:"Commit 4 files in `refact-lsp`",link_goto:"LINKS_AGAIN",link_tooltip:`git commmit -m "Add build script and test files for Docker image deployment and output generation..."
A build-remote.sh
A long-array.py
A long-output.py
A test.py`,link_payload:{project_path:"file:///Users/humbertoyusta/refact/refact-lsp",commit_message:"Add build script and test files for Docker image deployment and output generation\n\nIntroduced `build-remote.sh` to streamline the process of building a Docker image and deploying it to a remote server. This script automates the image creation, temporary container management, and binary transfer steps, improving efficiency and reducing manual errors. Additionally, added `long-array.py`, `long-output.py`, and `test.py` to facilitate testing and output generation scenarios, ensuring the system can handle large data sets and multiple output streams effectively.",file_changes:[{path:"build-remote.sh",status:"ADDED"},{path:"long-array.py",status:"ADDED"},{path:"long-output.py",status:"ADDED"},{path:"test.py",status:"ADDED"}]}}]},O="941fb8f4-409c-4430-a3b2-6450fafdb9f4",$e={current_thread_id:O,open_thread_ids:[O],threads:{[O]:{thread:{mode:"CONFIGURE",id:O,messages:[{role:"user",content:`🔧 The postgres tool should be visible now. To test the tool, list the tables available, briefly desctibe the tables and express
satisfaction and relief if it works, and change nothing. If it doesn't work or the tool isn't available, go through the usual plan in the system prompt.
The current config file is .
`},{role:"assistant",content:"",tool_calls:[{id:"call_IkNfXpwhNVR6D1Sr2CDA5Cfi",function:{arguments:`{"query":"\\nSELECT table_name\\nFROM information_schema.tables\\nWHERE table_schema='public';"}`,name:"postgres"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_IkNfXpwhNVR6D1Sr2CDA5Cfi",content:`🧩 for configuration go to SETTINGS:postgres, psql failed:
No such file or directory (os error 2)`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_kw6TJChemYjXEGL9mLL3T0mN",function:{arguments:'{"path":"/Users/marc/Projects/refact-lsp"}',name:"tree"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_kw6TJChemYjXEGL9mLL3T0mN",content:`/
  Users/
    marc/
      Projects/
        refact-lsp/
          .dockerignore
          .gitattributes
          .gitignore
          CODE_OF_CONDUCT.md
          CONTRIBUTING.md
          Cargo.lock
          Cargo.toml
          Cross.toml
          INTEGRATIONS.md
          LICENSE
          README.md
          build.rs
          tests/
            __init__.py
            lsp_connect.py
            test01_completion_edge_cases.py
            test02_completion_with_rag.py
            test03_at_commands_completion.py
            test04_completion_lsp.py
            test05_is_openai_compatible.py
            test06_tool_not_tool.py
            test07_memories.py
            test08_post_processing.py
            test09_ast_pick_up_changes.py
            test10_locate.py
            test11_patch.py
            test11_patch_partial_edit.py
            test12_tools_authorize_calls.py
            test13_vision.py
            test_diff_handlers.py
            test13_data/
              200.jpg
              530.jpg
            test11_data/
              already_applied_rewrite_symbol_01.py
              already_applied_rewrite_symbol_02.py
              toad_orig.py
              toad_partial_edit_01.py
              toad_partial_edit_02.py
              toad_rewrite_symbol_01.py
              toad_rewrite_symbol_02.py
              toad_rewrite_symbol_03.py
              toad_rewrite_symbol_04_orig.rs
              toad_rewrite_symbol_04_patched.rs
            emergency_frog_situation/
              frog.py
              holiday.py
              jump_to_conclusions.py
              set_as_avatar.py
              work_day.py
          src/
            background_tasks.rs
            cached_tokenizers.rs
            call_validation.rs
            caps.rs
            completion_cache.rs
            custom_error.rs
            diffs.rs
            fetch_embedding.rs
            file_filter.rs
            files_correction.rs
            files_in_jsonl.rs
            files_in_workspace.rs
            forward_to_hf_endpoint.rs
            forward_to_openai_endpoint.rs
            fuzzy_search.rs
            git.rs
            global_context.rs
            http.rs
            knowledge.rs
            known_models.rs
            lsp.rs
            main.rs
            nicer_logs.rs
            privacy.rs
            privacy_compiled_in.rs
            restream.rs
            scratchpad_abstract.rs
            subchat.rs
            version.rs
            yaml_configs/
              create_configs.rs
              customization_compiled_in.rs
              customization_loader.rs
              mod.rs
            vecdb/
              mod.rs
              vdb_cache.rs
              vdb_file_splitter.rs
              vdb_highlev.rs
              vdb_lance.rs
              vdb_remote.rs
              vdb_removed
              vdb_thread.rs
            tools/
              mod.rs
              tool_ast_definition.rs
              tool_ast_reference.rs
              tool_cat.rs
              tool_cmdline.rs
              tool_deep_thinking.rs
              tool_knowledge.rs
              tool_locate_search.rs
              tool_patch.rs
              tool_relevant_files.rs
              tool_search.rs
              tool_tree.rs
              tool_web.rs
              tools_description.rs
              tools_execute.rs
              tool_patch_aux/
                ast_lint.rs
                diff_apply.rs
                diff_removed
                fs_utils.rs
                mod.rs
                no_model_edit.rs
                postprocessing_utils.rs
                tickets_parsing.rs
                model_based_edit/
                  blocks_of_code_parser.rs
                  mod.rs
                  model_execution.rs
                  partial_edit.rs
                  whole_file_parser.rs
            removed
              removed
              removed
              removed
              removed
              mod.rs
              removed
              removed
              removed
              utils.rs
            scratchpads/
              chat_generic.rs
              chat_llama2.rs
              chat_passthrough.rs
              chat_utils_deltadelta.rs
              chat_utils_limit_history.rs
              chat_utils_prompts.rs
              code_completion_fim.rs
              code_completion_replace.rs
              comments_parser.rs
              mod.rs
              multimodality.rs
              passthrough_convert_messages.rs
              scratchpad_utils.rs
            postprocessing/
              mod.rs
              pp_command_output.rs
              pp_context_files.rs
              pp_plain_text.rs
              pp_utils.rs
            integrations/
              config_chat.rs
              integr_abstract.rs
              integr_chrome.rs
              integr_github.rs
              integr_gitlab.rs
              integr_pdb.rs
              integr_postgres.rs
              mod.rs
              process_io_utils.rs
              running_integrations.rs
              sessions.rs
              setting_up_integrations.rs
              yaml_schema.rs
              docker/
                docker_container_manager.rs
                docker_ssh_tunnel_utils.rs
                integr_docker.rs
                mod.rs
            http/
              routers.rs
              utils.rs
              routers/
                info.rs
                v1.rs
                v1/
                  ast.rs
                  at_commands.rs
                  at_tools.rs
                  caps.rs
                  chat.rs
                  code_completion.rs
                  code_lens.rs
                  customization.rs
                  removed
                  docker.rs
                  git.rs
                  graceful_shutdown.rs
                  gui_help_handlers.rs
                  handlers_memdb.rs
                  links.rs
                  lsp_like_handlers.rs
                  patch.rs
                  removed
                  status.rs
                  subchat.rs
                  sync_files.rs
                  system_prompt.rs
                  removed
                  v1_integrations.rs
                  vecdb.rs
            removed
              removed
              mod.rs
              removed
              utils.rs
            at_commands/
              at_ast_definition.rs
              at_ast_reference.rs
              at_commands.rs
              at_file.rs
              at_search.rs
              at_tree.rs
              at_web.rs
              execute_at.rs
              mod.rs
            ast/
              ast_db.rs
              ast_indexer_thread.rs
              ast_parse_anything.rs
              ast_removed
              chunk_utils.rs
              dummy_tokenizer.json
              file_splitter.rs
              linters.rs
              mod.rs
              parse_common.rs
              parse_python.rs
              treesitter/
                ast_instance_removed
                file_ast_markup.rs
                language_id.rs
                mod.rs
                parsers.rs
                skeletonizer.rs
                removed
                parsers/
                  cpp.rs
                  java.rs
                  js.rs
                  python.rs
                  rust.rs
                  tests.rs
                  ts.rs
                  utils.rs
                  tests/
                    cpp.rs
                    java.rs
                    js.rs
                    python.rs
                    rust.rs
                    ts.rs
                    cases/
                      ts/
                        main.ts
                        main.ts.json
                        person.ts
                        person.ts.decl_json
                        person.ts.skeleton
                      rust/
                        main.rs
                        main.rs.json
                        point.rs
                        point.rs.decl_json
                        point.rs.skeleton
                      python/
                        calculator.py
                        calculator.py.decl_json
                        calculator.py.skeleton
                        main.py
                        main.py.json
                      js/
                        car.js
                        car.js.decl_json
                        car.js.skeleton
                        main.js
                        main.js.json
                      java/
                        main.java
                        main.java.json
                        person.java
                        person.java.decl_json
                        person.java.skeleton
                      cpp/
                        circle.cpp
                        circle.cpp.decl_json
                        circle.cpp.skeleton
                        main.cpp
                        main.cpp.json
              alt_testsuite/
                cpp_goat_library.correct
                cpp_goat_library.h
                cpp_goat_main.correct
                cpp_goat_main.cpp
                jump_to_conclusions_annotated.py
                py_goat_library.correct
                py_goat_library.py
                py_goat_library_annotated.py
                py_goat_main.py
                py_goat_main_annotated.py
                py_torture1_attr.py
                py_torture1_attr_annotated.py
                py_torture2_resolving.py
                py_torture2_resolving_annotated.py
          examples/
            ast_definition.sh
            ast_references.sh
            chat_with_at_command.py
            http_caps.sh
            http_chat.sh
            http_chat_passthrough.sh
            http_completion.sh
            http_rag_status.sh
            http_subchat.sh
            http_vecdb_search.sh
            lsp_runner.py
            note3.py
            rag_skeletonize_video.py
          docker/
            lsp-debug.Dockerfile
          chrome/
            mac_arm-130.0.6723.69/
              chrome-mac-arm64/
                Google Chrome for Testing.app/
                  Contents/
                    Resources/
                      com.google.chrome.for.testing.manifest/
                        Contents/
                          Resources/
                            com.google.chrome.for.testing.manifest
                            en.lproj/
                              Localizable.strings
                    Frameworks/
                      Google Chrome for Testing Framework.framework/
                        Versions/
                          130.0.6723.69/
                            Libraries/
                              WidevineCdm/
                                _platform_specific/
                                  mac_arm64/
                                    libwidevinecdm.dylib
          bring_your_own_key/
            hf.yaml
            mixed.yaml
            openai.yaml
            openrouter.yaml`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_4we2wH5H50A2m6CIJqKbzYH8",function:{arguments:'{"paths": "Cargo.toml", "symbols": ""}',name:"cat"},type:"function",index:0},{id:"call_vmIGl31ytfpLWPkc138HJnxz",function:{arguments:'{"paths": "README.md", "symbols": ""}',name:"cat"},type:"function",index:1},{id:"call_QD1oyHwPOvvFdYUfV3ijiKzB",function:{arguments:'{"paths": "tests/emergency_frog_situation/frog.py", "symbols": ""}',name:"cat"},type:"function",index:2}]},{role:"tool",tool_call_id:"call_QD1oyHwPOvvFdYUfV3ijiKzB",content:`Paths found:
/Users/marc/Projects/refact-lsp/tests/emergency_frog_situation/frog.py
`,tool_failed:!1},{role:"tool",tool_call_id:"call_vmIGl31ytfpLWPkc138HJnxz",content:`Path problems:

The path "README.md" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"tool",tool_call_id:"call_4we2wH5H50A2m6CIJqKbzYH8",content:`Path problems:

The path "Cargo.toml" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"context_file",content:[{file_name:"frog.py",file_content:`import numpy as np

DT = 0.01

class Frog:
    def __init__(self, x, y, vx, vy):
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy

    def bounce_off_banks(self, pond_width, pond_height):
        if self.x < 0:
            self.vx = np.abs(self.vx)
        elif self.x > pond_width:
            self.vx = -np.abs(self.vx)
        if self.y < 0:
            self.vy = np.abs(self.vy)
        elif self.y > pond_height:
            self.vy = -np.abs(self.vy)

    def jump(self, pond_width, pond_height):
        self.x += self.vx * DT
        self.y += self.vy * DT
        self.bounce_off_banks(pond_width, pond_height)
        self.x = np.clip(self.x, 0, pond_width)
        self.y = np.clip(self.y, 0, pond_height)

    def croak(self, n_times):
        for n in range(n_times):
            print("croak")


class AlternativeFrog:
    def alternative_jump(self):
        pass


def standalone_jumping_function():
    print("I'm a frog! Jump! Jump! Really!")
`,line1:1,line2:39}]},{role:"assistant",content:"",tool_calls:[{id:"call_sNnvyOLlmulfHIikJzKhmz55",function:{arguments:'{"paths": "src/Cargo.toml", "symbols": ""}',name:"cat"},type:"function",index:0},{id:"call_n9FACVDl6a4tllgyWaLCl8s9",function:{arguments:'{"paths": "src/main.rs", "symbols": ""}',name:"cat"},type:"function",index:1},{id:"call_1DUkSfLq6Io8xowqVjJEuybM",function:{arguments:'{"paths": "src/lib.rs", "symbols": ""}',name:"cat"},type:"function",index:2}]},{role:"tool",tool_call_id:"call_1DUkSfLq6Io8xowqVjJEuybM",content:`Path problems:

The path "src/lib.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"tool",tool_call_id:"call_n9FACVDl6a4tllgyWaLCl8s9",content:`Path problems:

The path "src/main.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"tool",tool_call_id:"call_sNnvyOLlmulfHIikJzKhmz55",content:`Path problems:

The path "src/Cargo.toml" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_h8biZ8JNUjeWGbL9CCatbPcf",function:{arguments:'{"paths":"src/background_tasks.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_h8biZ8JNUjeWGbL9CCatbPcf",content:`Path problems:

The path "src/background_tasks.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_coI5PDRF2O2Mjcfb8bdZ4Heo",function:{arguments:'{"paths":"src/tools/integr_postgres.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_coI5PDRF2O2Mjcfb8bdZ4Heo",content:`Path problems:

The path "src/tools/integr_postgres.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_RXDRwr3yCJEplHud4xRoxWlC",function:{arguments:'{"paths":"src/integrations/integr_postgres.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_RXDRwr3yCJEplHud4xRoxWlC",content:`Path problems:

The path "src/integrations/integr_postgres.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_OvYAforNKAFz0d7mv96AnTVR",function:{arguments:'{"paths":"src/integrations/integr_abstract.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_OvYAforNKAFz0d7mv96AnTVR",content:`Path problems:

The path "src/integrations/integr_abstract.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_3wuMILBYl0z7pBMUoJOwohQf",function:{arguments:'{"paths":"src/integrations/integr_github.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_3wuMILBYl0z7pBMUoJOwohQf",content:`Path problems:

The path "src/integrations/integr_github.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_AzBRj8RiL1wc8eejuBVdaS6t",function:{arguments:'{"paths":"src/integrations/integr_gitlab.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_AzBRj8RiL1wc8eejuBVdaS6t",content:`Path problems:

The path "src/integrations/integr_gitlab.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_bJ2MrMSJHk4IF6Gp5DNLhJZP",function:{arguments:'{"paths":"src/integrations/integr_chrome.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_bJ2MrMSJHk4IF6Gp5DNLhJZP",content:`Path problems:

The path "src/integrations/integr_chrome.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_kZFTzONZdyo11FQcSudIo3vK",function:{arguments:'{"paths":"src/integrations/integr_pdb.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_kZFTzONZdyo11FQcSudIo3vK",content:`Path problems:

The path "src/integrations/integr_pdb.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_MDynldaxbGEuCKSuQg0Vgk5z",function:{arguments:'{"paths":"src/integrations/integr_docker.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_MDynldaxbGEuCKSuQg0Vgk5z",content:`Path problems:

The path "src/integrations/integr_docker.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_9fCGZwstx7G1MgHs6JD5JWTn",function:{arguments:'{"paths":"src/integrations/integr_abstract.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_9fCGZwstx7G1MgHs6JD5JWTn",content:`Path problems:

The path "src/integrations/integr_abstract.rs" does not exist. There are no similar names either.
`,tool_failed:!1},{role:"assistant",content:"",tool_calls:[{id:"call_etmMcI1UwBSaWwZHzxsuL8xu",function:{arguments:'{"paths":"src/integrations/integr_postgres.rs","symbols":""}',name:"cat"},type:"function",index:0}]},{role:"tool",tool_call_id:"call_etmMcI1UwBSaWwZHzxsuL8xu",content:`Path problems:

The path "src/integrations/integr_postgres.rs" does not exist. There are no similar names either.
`,tool_failed:!1}],title:`🔧 The postgres tool should be visible now. To test the tool, list the tables available, briefly desctibe the tables and express
satisfaction and relief if it works, and change nothing. If it doesn't work or the tool isn't available, go through the usual plan in the system prompt.
The current config file is .
`,model:"",tool_use:"agent",integration:{name:"postgres",path:"",project:""},new_chat_suggested:{wasSuggested:!1},createdAt:"2024-12-02T14:42:18.902Z",updatedAt:"2024-12-02T14:42:18.902Z"},streaming:!1,waiting_for_response:!1,prevent_send:!0,error:null,queued_items:[],send_immediately:!1,attached_images:[],attached_text_files:[],background_agents:{},confirmation:{pause:!1,pause_reasons:[],status:{wasInteracted:!1,confirmationStatus:!0}},memory_enrichment_user_touched:!1,manual_preview_items:[],manual_preview_ran:!1,snapshot_received:!0,task_widget_expanded:!1}},max_new_tokens:4096,system_prompt:{},tool_use:"agent",sse_refresh_requested:null,stream_version:0};function oe(){K(typeof URL<"u",L.formatMessage(`Global "URL" class is not defined. This likely means that you're running MSW in an environment that doesn't support all Node.js standard API (e.g. React Native). If that's the case, please use an appropriate polyfill for the "URL" class, like "react-native-url-polyfill".`))}function se(e,n){return e.toLowerCase()===n.toLowerCase()}function ie(e){return e<300?"#69AB32":e<400?"#F0BB4B":"#E95F5D"}async function le(e){const t=await e.clone().text();return{url:new URL(e.url),method:e.method,headers:Object.fromEntries(e.headers.entries()),body:t}}const{message:ce}=Y;async function pe(e){const n=e.clone(),t=await n.text(),r=n.status||200,a=n.statusText||ce[r]||"OK";return{status:r,statusText:a,headers:Object.fromEntries(n.headers.entries()),body:t}}function _e(e){for(var n=[],t=0;t<e.length;){var r=e[t];if(r==="*"||r==="+"||r==="?"){n.push({type:"MODIFIER",index:t,value:e[t++]});continue}if(r==="\\"){n.push({type:"ESCAPED_CHAR",index:t++,value:e[t++]});continue}if(r==="{"){n.push({type:"OPEN",index:t,value:e[t++]});continue}if(r==="}"){n.push({type:"CLOSE",index:t,value:e[t++]});continue}if(r===":"){for(var a="",o=t+1;o<e.length;){var i=e.charCodeAt(o);if(i>=48&&i<=57||i>=65&&i<=90||i>=97&&i<=122||i===95){a+=e[o++];continue}break}if(!a)throw new TypeError("Missing parameter name at ".concat(t));n.push({type:"NAME",index:t,value:a}),t=o;continue}if(r==="("){var m=1,x="",o=t+1;if(e[o]==="?")throw new TypeError('Pattern cannot start with "?" at '.concat(o));for(;o<e.length;){if(e[o]==="\\"){x+=e[o++]+e[o++];continue}if(e[o]===")"){if(m--,m===0){o++;break}}else if(e[o]==="("&&(m++,e[o+1]!=="?"))throw new TypeError("Capturing groups are not allowed at ".concat(o));x+=e[o++]}if(m)throw new TypeError("Unbalanced pattern at ".concat(t));if(!x)throw new TypeError("Missing pattern at ".concat(t));n.push({type:"PATTERN",index:t,value:x}),t=o;continue}n.push({type:"CHAR",index:t,value:e[t++]})}return n.push({type:"END",index:t,value:""}),n}function me(e,n){n===void 0&&(n={});for(var t=_e(e),r=n.prefixes,a=r===void 0?"./":r,o=n.delimiter,i=o===void 0?"/#?":o,m=[],x=0,b=0,y="",u=function(g){if(b<t.length&&t[b].type===g)return t[b++].value},C=function(g){var d=u(g);if(d!==void 0)return d;var T=t[b],I=T.type,W=T.index;throw new TypeError("Unexpected ".concat(I," at ").concat(W,", expected ").concat(g))},v=function(){for(var g="",d;d=u("CHAR")||u("ESCAPED_CHAR");)g+=d;return g},l=function(g){for(var d=0,T=i;d<T.length;d++){var I=T[d];if(g.indexOf(I)>-1)return!0}return!1},f=function(g){var d=m[m.length-1],T=g||(d&&typeof d=="string"?d:"");if(d&&!T)throw new TypeError('Must have text between two parameters, missing text after "'.concat(d.name,'"'));return!T||l(T)?"[^".concat(j(i),"]+?"):"(?:(?!".concat(j(T),")[^").concat(j(i),"])+?")};b<t.length;){var s=u("CHAR"),_=u("NAME"),k=u("PATTERN");if(_||k){var c=s||"";a.indexOf(c)===-1&&(y+=c,c=""),y&&(m.push(y),y=""),m.push({name:_||x++,prefix:c,suffix:"",pattern:k||f(c),modifier:u("MODIFIER")||""});continue}var p=s||u("ESCAPED_CHAR");if(p){y+=p;continue}y&&(m.push(y),y="");var h=u("OPEN");if(h){var c=v(),w=u("NAME")||"",E=u("PATTERN")||"",R=v();C("CLOSE"),m.push({name:w||(E?x++:""),pattern:w&&!E?f(c):E,prefix:c,suffix:R,modifier:u("MODIFIER")||""});continue}C("END")}return m}function ue(e,n){var t=[],r=F(e,t,n);return fe(r,t,n)}function fe(e,n,t){t===void 0&&(t={});var r=t.decode,a=r===void 0?function(o){return o}:r;return function(o){var i=e.exec(o);if(!i)return!1;for(var m=i[0],x=i.index,b=Object.create(null),y=function(C){if(i[C]===void 0)return"continue";var v=n[C-1];v.modifier==="*"||v.modifier==="+"?b[v.name]=i[C].split(v.prefix+v.suffix).map(function(l){return a(l,v)}):b[v.name]=a(i[C],v)},u=1;u<i.length;u++)y(u);return{path:m,index:x,params:b}}}function j(e){return e.replace(/([.+*?=^!:${}()[\]|/\\])/g,"\\$1")}function M(e){return e&&e.sensitive?"":"i"}function de(e,n){if(!n)return e;for(var t=/\((?:\?<(.*?)>)?(?!\?)/g,r=0,a=t.exec(e.source);a;)n.push({name:a[1]||r++,prefix:"",suffix:"",modifier:"",pattern:""}),a=t.exec(e.source);return e}function he(e,n,t){var r=e.map(function(a){return F(a,n,t).source});return new RegExp("(?:".concat(r.join("|"),")"),M(t))}function ge(e,n,t){return ye(me(e,t),n,t)}function ye(e,n,t){t===void 0&&(t={});for(var r=t.strict,a=r===void 0?!1:r,o=t.start,i=o===void 0?!0:o,m=t.end,x=m===void 0?!0:m,b=t.encode,y=b===void 0?function(d){return d}:b,u=t.delimiter,C=u===void 0?"/#?":u,v=t.endsWith,l=v===void 0?"":v,f="[".concat(j(l),"]|$"),s="[".concat(j(C),"]"),_=i?"^":"",k=0,c=e;k<c.length;k++){var p=c[k];if(typeof p=="string")_+=j(y(p));else{var h=j(y(p.prefix)),w=j(y(p.suffix));if(p.pattern)if(n&&n.push(p),h||w)if(p.modifier==="+"||p.modifier==="*"){var E=p.modifier==="*"?"?":"";_+="(?:".concat(h,"((?:").concat(p.pattern,")(?:").concat(w).concat(h,"(?:").concat(p.pattern,"))*)").concat(w,")").concat(E)}else _+="(?:".concat(h,"(").concat(p.pattern,")").concat(w,")").concat(p.modifier);else{if(p.modifier==="+"||p.modifier==="*")throw new TypeError('Can not repeat "'.concat(p.name,'" without a prefix and suffix'));_+="(".concat(p.pattern,")").concat(p.modifier)}else _+="(?:".concat(h).concat(w,")").concat(p.modifier)}}if(x)a||(_+="".concat(s,"?")),_+=t.endsWith?"(?=".concat(f,")"):"$";else{var R=e[e.length-1],g=typeof R=="string"?s.indexOf(R[R.length-1])>-1:R===void 0;a||(_+="(?:".concat(s,"(?=").concat(f,"))?")),g||(_+="(?=".concat(s,"|").concat(f,")"))}return new RegExp(_,M(t))}function F(e,n,t){return e instanceof RegExp?de(e,n):Array.isArray(e)?he(e,n,t):ge(e,n,t)}function ve(e,n=!0){return[n&&e.origin,e.pathname].filter(Boolean).join("")}const be=/[\?|#].*$/g;function xe(e){return new URL(`/${e}`,"http://localhost").searchParams}function J(e){return e.endsWith("?")?e:e.replace(be,"")}function we(e){return/^([a-z][a-z\d\+\-\.]*:)?\/\//i.test(e)}function Te(e,n){if(we(e)||e.startsWith("*"))return e;const t=n||typeof location<"u"&&location.href;return t?decodeURI(new URL(encodeURI(e),t).href):e}function Ce(e,n){if(e instanceof RegExp)return e;const t=Te(e,n);return J(t)}function ke(e){return e.replace(/([:a-zA-Z_-]*)(\*{1,2})+/g,(n,t,r)=>{const a="(.*)";return t?t.startsWith(":")?`${t}${r}`:`${t}${a}`:a}).replace(/([^\/])(:)(?=\d+)/,"$1\\$2").replace(/^([^\/]+)(:)(?=\/\/)/,"$1\\$2")}function Ee(e,n,t){const r=Ce(n,t),a=typeof r=="string"?ke(r):r,o=ve(e),i=ue(a,{decode:decodeURIComponent})(o),m=i&&i.params||{};return{matches:i!==!1,params:m}}var Re=Object.create,$=Object.defineProperty,Ae=Object.getOwnPropertyDescriptor,G=Object.getOwnPropertyNames,Pe=Object.getPrototypeOf,je=Object.prototype.hasOwnProperty,Se=(e,n)=>function(){return n||(0,e[G(e)[0]])((n={exports:{}}).exports,n),n.exports},De=(e,n,t,r)=>{if(n&&typeof n=="object"||typeof n=="function")for(let a of G(n))!je.call(e,a)&&a!==t&&$(e,a,{get:()=>n[a],enumerable:!(r=Ae(n,a))||r.enumerable});return e},Oe=(e,n,t)=>(t=e!=null?Re(Pe(e)):{},De($(t,"default",{value:e,enumerable:!0}),e)),Ie=Se({"node_modules/cookie/index.js"(e){e.parse=m,e.serialize=y;var n=Object.prototype.toString,t=Object.prototype.hasOwnProperty,r=/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/,a=/^("?)[\u0021\u0023-\u002B\u002D-\u003A\u003C-\u005B\u005D-\u007E]*\1$/,o=/^([.]?[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)([.][a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/i,i=/^[\u0020-\u003A\u003D-\u007E]*$/;function m(l,f){if(typeof l!="string")throw new TypeError("argument str must be a string");var s={},_=l.length;if(_<2)return s;var k=f&&f.decode||u,c=0,p=0,h=0;do{if(p=l.indexOf("=",c),p===-1)break;if(h=l.indexOf(";",c),h===-1)h=_;else if(p>h){c=l.lastIndexOf(";",p-1)+1;continue}var w=x(l,c,p),E=b(l,p,w),R=l.slice(w,E);if(!t.call(s,R)){var g=x(l,p+1,h),d=b(l,h,g);l.charCodeAt(g)===34&&l.charCodeAt(d-1)===34&&(g++,d--);var T=l.slice(g,d);s[R]=v(T,k)}c=h+1}while(c<_);return s}function x(l,f,s){do{var _=l.charCodeAt(f);if(_!==32&&_!==9)return f}while(++f<s);return s}function b(l,f,s){for(;f>s;){var _=l.charCodeAt(--f);if(_!==32&&_!==9)return f+1}return s}function y(l,f,s){var _=s&&s.encode||encodeURIComponent;if(typeof _!="function")throw new TypeError("option encode is invalid");if(!r.test(l))throw new TypeError("argument name is invalid");var k=_(f);if(!a.test(k))throw new TypeError("argument val is invalid");var c=l+"="+k;if(!s)return c;if(s.maxAge!=null){var p=Math.floor(s.maxAge);if(!isFinite(p))throw new TypeError("option maxAge is invalid");c+="; Max-Age="+p}if(s.domain){if(!o.test(s.domain))throw new TypeError("option domain is invalid");c+="; Domain="+s.domain}if(s.path){if(!i.test(s.path))throw new TypeError("option path is invalid");c+="; Path="+s.path}if(s.expires){var h=s.expires;if(!C(h)||isNaN(h.valueOf()))throw new TypeError("option expires is invalid");c+="; Expires="+h.toUTCString()}if(s.httpOnly&&(c+="; HttpOnly"),s.secure&&(c+="; Secure"),s.partitioned&&(c+="; Partitioned"),s.priority){var w=typeof s.priority=="string"?s.priority.toLowerCase():s.priority;switch(w){case"low":c+="; Priority=Low";break;case"medium":c+="; Priority=Medium";break;case"high":c+="; Priority=High";break;default:throw new TypeError("option priority is invalid")}}if(s.sameSite){var E=typeof s.sameSite=="string"?s.sameSite.toLowerCase():s.sameSite;switch(E){case!0:c+="; SameSite=Strict";break;case"lax":c+="; SameSite=Lax";break;case"strict":c+="; SameSite=Strict";break;case"none":c+="; SameSite=None";break;default:throw new TypeError("option sameSite is invalid")}}return c}function u(l){return l.indexOf("%")!==-1?decodeURIComponent(l):l}function C(l){return n.call(l)==="[object Date]"}function v(l,f){try{return f(l)}catch{return l}}}}),Le=Oe(Ie()),q=Le.default;/*! Bundled license information:

cookie/index.js:
  (*!
   * cookie
   * Copyright(c) 2012-2014 Roman Shtylman
   * Copyright(c) 2015 Douglas Christopher Wilson
   * MIT Licensed
   *)
*/function B(e){const n=q.parse(e),t={};for(const r in n)typeof n[r]<"u"&&(t[r]=n[r]);return t}function U(){return B(document.cookie)}function Ne(e){if(typeof document>"u"||typeof location>"u")return{};switch(e.credentials){case"same-origin":{const n=new URL(e.url);return location.origin===n.origin?U():{}}case"include":return U();default:return{}}}function Ue(e){const n=e.headers.get("cookie"),t=n?B(n):{},r=Ne(e);for(const i in r)e.headers.append("cookie",q.serialize(i,r[i]));const a=Q.getCookiesSync(e.url),o=Object.fromEntries(a.map(i=>[i.key,i.value]));for(const i of a)e.headers.append("cookie",i.toString());return{...r,...o,...t}}var P=(e=>(e.HEAD="HEAD",e.GET="GET",e.POST="POST",e.PUT="PUT",e.PATCH="PATCH",e.OPTIONS="OPTIONS",e.DELETE="DELETE",e))(P||{});class ze extends X{constructor(n,t,r,a){super({info:{header:`${n} ${t}`,path:t,method:n},resolver:r,options:a}),this.checkRedundantQueryParameters()}checkRedundantQueryParameters(){const{method:n,path:t}=this.info;if(t instanceof RegExp||J(t)===t)return;xe(t).forEach((o,i)=>{}),L.warn(`Found a redundant usage of query parameters in the request handler URL for "${n} ${t}". Please match against a path instead and access query parameters using "new URL(request.url).searchParams" instead. Learn more: https://mswjs.io/docs/recipes/query-parameters`)}async parse(n){var o;const t=new URL(n.request.url),r=Ee(t,this.info.path,(o=n.resolutionContext)==null?void 0:o.baseUrl),a=Ue(n.request);return{match:r,cookies:a}}predicate(n){const t=this.matchMethod(n.request.method),r=n.parsedResult.match.matches;return t&&r}matchMethod(n){return this.info.method instanceof RegExp?this.info.method.test(n):se(this.info.method,n)}extendResolverArgs(n){var t;return{params:((t=n.parsedResult.match)==null?void 0:t.params)||{},cookies:n.parsedResult.cookies}}async log(n){const t=ee(n.request.url),r=await le(n.request),a=await pe(n.response),o=ie(a.status);console.groupCollapsed(L.formatMessage(`${te()} ${n.request.method} ${t} (%c${a.status} ${a.statusText}%c)`),`color:${o}`,"color:inherit"),console.log("Request",r),console.log("Handler:",this),console.log("Response",a),console.groupEnd()}}function A(e){return(n,t,r={})=>new ze(e,n,t,r)}const Ge={all:A(/.+/),head:A(P.HEAD),get:A(P.GET),post:A(P.POST),put:A(P.PUT),delete:A(P.DELETE),patch:A(P.PATCH),options:A(P.OPTIONS)},He=Symbol("bodyType");var z,H;class D extends(H=ne,z=He,H){constructor(t,r){const a=S(r);super(t,a);N(this,z);ae(this,a)}static text(t,r){const a=S(r);return a.headers.has("Content-Type")||a.headers.set("Content-Type","text/plain"),a.headers.has("Content-Length")||a.headers.set("Content-Length",t?new Blob([t]).size.toString():"0"),new D(t,a)}static json(t,r){const a=S(r);a.headers.has("Content-Type")||a.headers.set("Content-Type","application/json");const o=JSON.stringify(t);return a.headers.has("Content-Length")||a.headers.set("Content-Length",o?new Blob([o]).size.toString():"0"),new D(o,a)}static xml(t,r){const a=S(r);return a.headers.has("Content-Type")||a.headers.set("Content-Type","text/xml"),new D(t,a)}static html(t,r){const a=S(r);return a.headers.has("Content-Type")||a.headers.set("Content-Type","text/html"),new D(t,a)}static arrayBuffer(t,r){const a=S(r);return a.headers.has("Content-Type")||a.headers.set("Content-Type","application/octet-stream"),t&&!a.headers.has("Content-Length")&&a.headers.set("Content-Length",t.byteLength.toString()),new D(t,a)}static formData(t,r){return new D(t,S(r))}}oe();export{$e as C,D as H,Je as S,Ge as h};
