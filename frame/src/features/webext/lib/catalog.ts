import * as m from "$shared/i18n/messages";

/** Extensions verified to work well in Quasar: in daily use, and in the
 *  real-extension suite (`cargo xtask webext-suite`), which also checks the
 *  ones that act on pages really do. "Get" opens their Chrome Web Store page. */
export type CatalogEntry = {
  id: string;
  name: string;
  blurb: () => string;
  /** The Web Store image path of its icon, fetched by the browser. */
  icon: string;
};

export type CatalogGroup = {
  title: () => string;
  entries: CatalogEntry[];
};

export const catalog: CatalogGroup[] = [
  {
    title: m.webext_group_passwords,
    entries: [
      {
        id: "aeblfdkhhhdcdjpifhhbdiojplfjncoa",
        name: "1Password",
        blurb: m.webext_catalog_1password,
        icon: "DftM3biDXufonobNWlKgO74qdJosSFjMay_Ku9aIinJwwmtp-hv3Psof4nxKp2mjf6Lgu-pHWPAaPXz7Rs3Uwzen",
      },
      {
        id: "nngceckbapebfimnlniiiahkandclblb",
        name: "Bitwarden",
        blurb: m.webext_catalog_bitwarden,
        icon: "J_l8abQyJgx7POjRoDfGaFYWFnYQNpRSy4kH5IlbwSdM-l_gZf2rJlk2NLSQTY8g-U2vrclpb0EZApHyOe6sjzbKcUc",
      },
      {
        id: "hdokiejnpimakedhajhdlcegeplioahd",
        name: "LastPass",
        blurb: m.webext_catalog_lastpass,
        icon: "2E-kdWD96KmPMAR7_XC6d_zWHio7n7skW8-9S5R42ZQvTX-djSt-qbwhuiM5znQ5Iqj9SWuP4fqfycgt_OD9aLWh",
      },
    ],
  },
  {
    title: m.webext_group_language,
    entries: [
      {
        id: "kbfnbcaeplbcioakkpcpgfkobkghlhen",
        name: "Grammarly",
        blurb: m.webext_catalog_grammarly,
        icon: "Ywdz5mn9q2Mx76DU45LSH-Pv5OGpqk8QAOY3lT1AWScMTZYQtAhqhVjtY5I2JZK530QIycLZooe2a0k3quGqYUaZ",
      },
      {
        id: "cofdbpoegempjloogbagkncekinflcnj",
        name: "DeepL",
        blurb: m.webext_catalog_deepl,
        icon: "iXeQzL_4icUm7QIqYFHfpm13HKecoVh8mbaop8_kqEnUiETbYf4wK0C2haqLYAjePlhOxKLjIHpxhH7ufB4KIcKVrA",
      },
      {
        id: "mgijmajocgfcbeboacabfgobmjgjcoja",
        name: "Google Dictionary",
        blurb: m.webext_catalog_google_dictionary,
        icon: "R2e7I-0MQgxeJ-dkkGfDxL2PWPcR3DTGhttSloJO70ax2N0TEtcT-AlmLcMAJZyHOhhqRTTpuJjO9qVs24DUDIMYVsk",
      },
    ],
  },
  {
    title: m.webext_group_browsing,
    entries: [
      {
        id: "eimadpbcbfnmbkopoojfekhnkhdbieeh",
        name: "Dark Reader",
        blurb: m.webext_catalog_dark_reader,
        icon: "T66wTLk-gpBBGsMm0SDJJ3VaI8YM0Utr8NaGCSANmXOfb84K-9GmyXORLKoslfxtasKtQ4spDCdq_zlp_t3QQ6SI0A",
      },
      {
        id: "dbepggeogbaibhgnhhndojpepiihcmeb",
        name: "Vimium",
        blurb: m.webext_catalog_vimium,
        icon: "xHL8Vg17H_mzExbnVNQTQpEukXEBf4YEdmvexjSi-JSmIV3v_F5mBCgdIbSq_sGLqbLe-mYnavUufUDudf56MHaUxg",
      },
      {
        id: "clngdbkpkpeebahjckkjfobafhncgmne",
        name: "Stylus",
        blurb: m.webext_catalog_stylus,
        icon: "2K8pc_5-2DkPam9b3oAWoITZ7IuIz68A5a8Ssg2_MNNHTPWPOPSBVTFdTmeVu9hi8GJxpKbvTekgwpeyGV6vXyBKH80",
      },
    ],
  },
  {
    title: m.webext_group_video,
    entries: [
      {
        id: "mnjggcdmjocbbbhaepdhchncahnbgone",
        name: "SponsorBlock",
        blurb: m.webext_catalog_sponsorblock,
        icon: "oSoXDpjLX_iytl11_ROa1thmFI0xPk9pL8ttEtnFkBI8Cie0Ge8KxVFaokgBRscvUR1cXH4bVeG_C_Fl6kBw3A3_",
      },
      {
        id: "gebbhagfogifgggkldgodflihgfeippi",
        name: "Return YouTube Dislike",
        blurb: m.webext_catalog_ryd,
        icon: "bVYgRXHiKIDU1EqkGv58alRhXu-SjSqi-I_yZHak8ZvZo_kYxMePoqa3pyIX931tzFkQ3b-EbxT7gSk8M1eOSdpDCQ8",
      },
      {
        id: "ponfpcnoihfmfllpaingbgckeeldkhle",
        name: "Enhancer for YouTube",
        blurb: m.webext_catalog_enhancer_youtube,
        icon: "6PBcKpsoS15e2SUqMi6_KGBHsnvUdaRrRYXkHM3zkn5Zzj8TAEJp1_RtykaCfn1DCmyH9PJOKHrMbmtAOnQqtAU8aLs",
      },
      {
        id: "nffaoalbilbmmfgbnbgppjihopabppdk",
        name: "Video Speed Controller",
        blurb: m.webext_catalog_video_speed,
        icon: "5yY1QQ__b0c2ZLmmi6mcdjX-9V3LAgXhgUaZtsICFTjeQV0S6xkVncmV99oEtU-H8WN8dZpWh_cycXSXoyffADsoYg",
      },
    ],
  },
  {
    title: m.webext_group_saving,
    entries: [
      {
        id: "ldgfbffkinooeloadekpmfoklnobpien",
        name: "Raindrop.io",
        blurb: m.webext_catalog_raindrop,
        icon: "GsyRyzu8R26g2Caw3_AEcZD7opQ2CADN9wOdPdq8-Rr-zBvLgC_BOCQrYUwECEQ-4iub4XOQw36QMyWhyBhMF6Vf",
      },
      {
        id: "knheggckgoiihginacbkhaalnibhilkk",
        name: "Notion Web Clipper",
        blurb: m.webext_catalog_notion,
        icon: "vJmcSHco6MjgE6fCMmyb173fOOscaS2mvVESMrB5y8CutzXRT7mSGCHH510UdpqY4Rbh8BWRWHxjigVR5eZOwtc_oa8",
      },
      {
        id: "pioclpoplcdbaefihamjohnefbikjilc",
        name: "Evernote Web Clipper",
        blurb: m.webext_catalog_evernote,
        icon: "SahOY_2p25P1sbFgMKeD-2UqdV4i0LsZ-Jvx9vCzGrjl_bTdgW4m2JvjGSG-nWZdHYuiOj_jb0SSh98G6A_KdIAqHg",
      },
      {
        id: "ekhagklcjbdpajgpjgmbionohlpdbjgc",
        name: "Zotero Connector",
        blurb: m.webext_catalog_zotero,
        icon: "HzPUK6_F5Y_uoIGRPkC3tKdP14BQSzW9MhP0eRZBb10bXFUvmBHWbzaa0V4xqU3qG8s0_jsqdWKY3n7x9QC4jh-dZCs",
      },
    ],
  },
  {
    title: m.webext_group_developers,
    entries: [
      {
        id: "hlepfoohegkhhmjieoechaddaejaokhf",
        name: "Refined GitHub",
        blurb: m.webext_catalog_refined_github,
        icon: "4N2wipmBVx1qK0R0E0XdADE31-8IuMylOtO9AyFopOA9i3IQKoCC5L4nYFDy55xpxpk6qKusHuqXyKJqvw8jcJaiqg",
      },
      {
        id: "bkhaagjahfmjljalopjnoealnfndnagc",
        name: "Octotree",
        blurb: m.webext_catalog_octotree,
        icon: "wafm5uFaPRSo1RHMbhcdEghFzTPUfYo5GosPmBhkdNuYlGz8WigoAQM-8lulzuhWQBGTbbUyRvfoyIMDypJzuAVZ",
      },
      {
        id: "gppongmhjkpfnbhagpmjfkannfbllamg",
        name: "Wappalyzer",
        blurb: m.webext_catalog_wappalyzer,
        icon: "YssvlGeiyyXJhYYV8Fh4aeW_kuQ7zC-ap7kygvyLT59TZwnFWYy-9aHCfXpyKgBEwJhFajvSUFqpjiG_42yfQXIw2Q",
      },
    ],
  },
];

export const storeListing = (id: string) => `https://chromewebstore.google.com/detail/${id}`;
