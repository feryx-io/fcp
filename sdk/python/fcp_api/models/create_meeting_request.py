from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.video_codec import VideoCodec
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateMeetingRequest")



@_attrs_define
class CreateMeetingRequest:
    """ Create meeting request body.

        Attributes:
            room_number (str):
            name (str):
            guest_pin (None | str | Unset):
            host_pin (None | str | Unset):
            require_guest_pin (bool | Unset):
            max_participants (int | Unset):
            wait_for_moderator (bool | Unset):
            enabled (bool | Unset):
            auto_record (bool | Unset):
            default_locked (bool | Unset):
            permanent (bool | Unset):
            owner_id (None | str | Unset):
            pin_prompt_sound (None | str | Unset):
            invalid_pin_sound (None | str | Unset):
            moh_sound (None | str | Unset):
            pin_prompt_tts (None | str | Unset):
            invalid_pin_tts (None | str | Unset):
            welcome_tts (None | str | Unset):
            play_entry_tone (bool | Unset):
            play_exit_tone (bool | Unset):
            join_sound (None | str | Unset):
            exit_sound (None | str | Unset):
            video_enabled (bool | Unset):
            video_layout (str | Unset):
            video_max_participants (int | Unset):
            video_max_resolution (str | Unset):
            video_fps (int | Unset):
            video_exclude_self (bool | Unset):
            video_codecs (list[VideoCodec] | Unset):
            video_passthrough (str | Unset):
            video_content_share (str | Unset):
            video_content_layout (str | Unset):
            video_mode (str | Unset): `mix` (the default) or `auto`.
            kind (str | Unset): `meeting` (the default) or `webinar`.
            panelist_pin (None | str | Unset): A webinar's PIN for panelists; it must differ from the others.
            panelists (list[str] | Unset): A webinar's panelists: users of the tenant, by username or id
                (kept by id).
            qa (None | str | Unset): Questions from the audience: `off`, `moderated` or `open`; unset is
                `moderated` in a webinar and `off` in a meeting.
            qa_anonymous (bool | Unset): Whether a question may be asked without the asker's name.
            chat (None | str | Unset): Chat in the room: `everyone`, `hosts` or `off`; unset is
                `everyone` in a meeting and `hosts` in a webinar.
     """

    room_number: str
    name: str
    guest_pin: None | str | Unset = UNSET
    host_pin: None | str | Unset = UNSET
    require_guest_pin: bool | Unset = UNSET
    max_participants: int | Unset = UNSET
    wait_for_moderator: bool | Unset = UNSET
    enabled: bool | Unset = UNSET
    auto_record: bool | Unset = UNSET
    default_locked: bool | Unset = UNSET
    permanent: bool | Unset = UNSET
    owner_id: None | str | Unset = UNSET
    pin_prompt_sound: None | str | Unset = UNSET
    invalid_pin_sound: None | str | Unset = UNSET
    moh_sound: None | str | Unset = UNSET
    pin_prompt_tts: None | str | Unset = UNSET
    invalid_pin_tts: None | str | Unset = UNSET
    welcome_tts: None | str | Unset = UNSET
    play_entry_tone: bool | Unset = UNSET
    play_exit_tone: bool | Unset = UNSET
    join_sound: None | str | Unset = UNSET
    exit_sound: None | str | Unset = UNSET
    video_enabled: bool | Unset = UNSET
    video_layout: str | Unset = UNSET
    video_max_participants: int | Unset = UNSET
    video_max_resolution: str | Unset = UNSET
    video_fps: int | Unset = UNSET
    video_exclude_self: bool | Unset = UNSET
    video_codecs: list[VideoCodec] | Unset = UNSET
    video_passthrough: str | Unset = UNSET
    video_content_share: str | Unset = UNSET
    video_content_layout: str | Unset = UNSET
    video_mode: str | Unset = UNSET
    kind: str | Unset = UNSET
    panelist_pin: None | str | Unset = UNSET
    panelists: list[str] | Unset = UNSET
    qa: None | str | Unset = UNSET
    qa_anonymous: bool | Unset = UNSET
    chat: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        room_number = self.room_number

        name = self.name

        guest_pin: None | str | Unset
        if isinstance(self.guest_pin, Unset):
            guest_pin = UNSET
        else:
            guest_pin = self.guest_pin

        host_pin: None | str | Unset
        if isinstance(self.host_pin, Unset):
            host_pin = UNSET
        else:
            host_pin = self.host_pin

        require_guest_pin = self.require_guest_pin

        max_participants = self.max_participants

        wait_for_moderator = self.wait_for_moderator

        enabled = self.enabled

        auto_record = self.auto_record

        default_locked = self.default_locked

        permanent = self.permanent

        owner_id: None | str | Unset
        if isinstance(self.owner_id, Unset):
            owner_id = UNSET
        else:
            owner_id = self.owner_id

        pin_prompt_sound: None | str | Unset
        if isinstance(self.pin_prompt_sound, Unset):
            pin_prompt_sound = UNSET
        else:
            pin_prompt_sound = self.pin_prompt_sound

        invalid_pin_sound: None | str | Unset
        if isinstance(self.invalid_pin_sound, Unset):
            invalid_pin_sound = UNSET
        else:
            invalid_pin_sound = self.invalid_pin_sound

        moh_sound: None | str | Unset
        if isinstance(self.moh_sound, Unset):
            moh_sound = UNSET
        else:
            moh_sound = self.moh_sound

        pin_prompt_tts: None | str | Unset
        if isinstance(self.pin_prompt_tts, Unset):
            pin_prompt_tts = UNSET
        else:
            pin_prompt_tts = self.pin_prompt_tts

        invalid_pin_tts: None | str | Unset
        if isinstance(self.invalid_pin_tts, Unset):
            invalid_pin_tts = UNSET
        else:
            invalid_pin_tts = self.invalid_pin_tts

        welcome_tts: None | str | Unset
        if isinstance(self.welcome_tts, Unset):
            welcome_tts = UNSET
        else:
            welcome_tts = self.welcome_tts

        play_entry_tone = self.play_entry_tone

        play_exit_tone = self.play_exit_tone

        join_sound: None | str | Unset
        if isinstance(self.join_sound, Unset):
            join_sound = UNSET
        else:
            join_sound = self.join_sound

        exit_sound: None | str | Unset
        if isinstance(self.exit_sound, Unset):
            exit_sound = UNSET
        else:
            exit_sound = self.exit_sound

        video_enabled = self.video_enabled

        video_layout = self.video_layout

        video_max_participants = self.video_max_participants

        video_max_resolution = self.video_max_resolution

        video_fps = self.video_fps

        video_exclude_self = self.video_exclude_self

        video_codecs: list[str] | Unset = UNSET
        if not isinstance(self.video_codecs, Unset):
            video_codecs = []
            for video_codecs_item_data in self.video_codecs:
                video_codecs_item = video_codecs_item_data.value
                video_codecs.append(video_codecs_item)



        video_passthrough = self.video_passthrough

        video_content_share = self.video_content_share

        video_content_layout = self.video_content_layout

        video_mode = self.video_mode

        kind = self.kind

        panelist_pin: None | str | Unset
        if isinstance(self.panelist_pin, Unset):
            panelist_pin = UNSET
        else:
            panelist_pin = self.panelist_pin

        panelists: list[str] | Unset = UNSET
        if not isinstance(self.panelists, Unset):
            panelists = self.panelists



        qa: None | str | Unset
        if isinstance(self.qa, Unset):
            qa = UNSET
        else:
            qa = self.qa

        qa_anonymous = self.qa_anonymous

        chat: None | str | Unset
        if isinstance(self.chat, Unset):
            chat = UNSET
        else:
            chat = self.chat


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "room_number": room_number,
            "name": name,
        })
        if guest_pin is not UNSET:
            field_dict["guest_pin"] = guest_pin
        if host_pin is not UNSET:
            field_dict["host_pin"] = host_pin
        if require_guest_pin is not UNSET:
            field_dict["require_guest_pin"] = require_guest_pin
        if max_participants is not UNSET:
            field_dict["max_participants"] = max_participants
        if wait_for_moderator is not UNSET:
            field_dict["wait_for_moderator"] = wait_for_moderator
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if auto_record is not UNSET:
            field_dict["auto_record"] = auto_record
        if default_locked is not UNSET:
            field_dict["default_locked"] = default_locked
        if permanent is not UNSET:
            field_dict["permanent"] = permanent
        if owner_id is not UNSET:
            field_dict["owner_id"] = owner_id
        if pin_prompt_sound is not UNSET:
            field_dict["pin_prompt_sound"] = pin_prompt_sound
        if invalid_pin_sound is not UNSET:
            field_dict["invalid_pin_sound"] = invalid_pin_sound
        if moh_sound is not UNSET:
            field_dict["moh_sound"] = moh_sound
        if pin_prompt_tts is not UNSET:
            field_dict["pin_prompt_tts"] = pin_prompt_tts
        if invalid_pin_tts is not UNSET:
            field_dict["invalid_pin_tts"] = invalid_pin_tts
        if welcome_tts is not UNSET:
            field_dict["welcome_tts"] = welcome_tts
        if play_entry_tone is not UNSET:
            field_dict["play_entry_tone"] = play_entry_tone
        if play_exit_tone is not UNSET:
            field_dict["play_exit_tone"] = play_exit_tone
        if join_sound is not UNSET:
            field_dict["join_sound"] = join_sound
        if exit_sound is not UNSET:
            field_dict["exit_sound"] = exit_sound
        if video_enabled is not UNSET:
            field_dict["video_enabled"] = video_enabled
        if video_layout is not UNSET:
            field_dict["video_layout"] = video_layout
        if video_max_participants is not UNSET:
            field_dict["video_max_participants"] = video_max_participants
        if video_max_resolution is not UNSET:
            field_dict["video_max_resolution"] = video_max_resolution
        if video_fps is not UNSET:
            field_dict["video_fps"] = video_fps
        if video_exclude_self is not UNSET:
            field_dict["video_exclude_self"] = video_exclude_self
        if video_codecs is not UNSET:
            field_dict["video_codecs"] = video_codecs
        if video_passthrough is not UNSET:
            field_dict["video_passthrough"] = video_passthrough
        if video_content_share is not UNSET:
            field_dict["video_content_share"] = video_content_share
        if video_content_layout is not UNSET:
            field_dict["video_content_layout"] = video_content_layout
        if video_mode is not UNSET:
            field_dict["video_mode"] = video_mode
        if kind is not UNSET:
            field_dict["kind"] = kind
        if panelist_pin is not UNSET:
            field_dict["panelist_pin"] = panelist_pin
        if panelists is not UNSET:
            field_dict["panelists"] = panelists
        if qa is not UNSET:
            field_dict["qa"] = qa
        if qa_anonymous is not UNSET:
            field_dict["qa_anonymous"] = qa_anonymous
        if chat is not UNSET:
            field_dict["chat"] = chat

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        room_number = d.pop("room_number")

        name = d.pop("name")

        def _parse_guest_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        guest_pin = _parse_guest_pin(d.pop("guest_pin", UNSET))


        def _parse_host_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        host_pin = _parse_host_pin(d.pop("host_pin", UNSET))


        require_guest_pin = d.pop("require_guest_pin", UNSET)

        max_participants = d.pop("max_participants", UNSET)

        wait_for_moderator = d.pop("wait_for_moderator", UNSET)

        enabled = d.pop("enabled", UNSET)

        auto_record = d.pop("auto_record", UNSET)

        default_locked = d.pop("default_locked", UNSET)

        permanent = d.pop("permanent", UNSET)

        def _parse_owner_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        owner_id = _parse_owner_id(d.pop("owner_id", UNSET))


        def _parse_pin_prompt_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        pin_prompt_sound = _parse_pin_prompt_sound(d.pop("pin_prompt_sound", UNSET))


        def _parse_invalid_pin_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        invalid_pin_sound = _parse_invalid_pin_sound(d.pop("invalid_pin_sound", UNSET))


        def _parse_moh_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        moh_sound = _parse_moh_sound(d.pop("moh_sound", UNSET))


        def _parse_pin_prompt_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        pin_prompt_tts = _parse_pin_prompt_tts(d.pop("pin_prompt_tts", UNSET))


        def _parse_invalid_pin_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        invalid_pin_tts = _parse_invalid_pin_tts(d.pop("invalid_pin_tts", UNSET))


        def _parse_welcome_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        welcome_tts = _parse_welcome_tts(d.pop("welcome_tts", UNSET))


        play_entry_tone = d.pop("play_entry_tone", UNSET)

        play_exit_tone = d.pop("play_exit_tone", UNSET)

        def _parse_join_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        join_sound = _parse_join_sound(d.pop("join_sound", UNSET))


        def _parse_exit_sound(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        exit_sound = _parse_exit_sound(d.pop("exit_sound", UNSET))


        video_enabled = d.pop("video_enabled", UNSET)

        video_layout = d.pop("video_layout", UNSET)

        video_max_participants = d.pop("video_max_participants", UNSET)

        video_max_resolution = d.pop("video_max_resolution", UNSET)

        video_fps = d.pop("video_fps", UNSET)

        video_exclude_self = d.pop("video_exclude_self", UNSET)

        _video_codecs = d.pop("video_codecs", UNSET)
        video_codecs: list[VideoCodec] | Unset = UNSET
        if _video_codecs is not UNSET:
            video_codecs = []
            for video_codecs_item_data in _video_codecs:
                video_codecs_item = VideoCodec(video_codecs_item_data)



                video_codecs.append(video_codecs_item)


        video_passthrough = d.pop("video_passthrough", UNSET)

        video_content_share = d.pop("video_content_share", UNSET)

        video_content_layout = d.pop("video_content_layout", UNSET)

        video_mode = d.pop("video_mode", UNSET)

        kind = d.pop("kind", UNSET)

        def _parse_panelist_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        panelist_pin = _parse_panelist_pin(d.pop("panelist_pin", UNSET))


        panelists = cast(list[str], d.pop("panelists", UNSET))


        def _parse_qa(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        qa = _parse_qa(d.pop("qa", UNSET))


        qa_anonymous = d.pop("qa_anonymous", UNSET)

        def _parse_chat(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        chat = _parse_chat(d.pop("chat", UNSET))


        create_meeting_request = cls(
            room_number=room_number,
            name=name,
            guest_pin=guest_pin,
            host_pin=host_pin,
            require_guest_pin=require_guest_pin,
            max_participants=max_participants,
            wait_for_moderator=wait_for_moderator,
            enabled=enabled,
            auto_record=auto_record,
            default_locked=default_locked,
            permanent=permanent,
            owner_id=owner_id,
            pin_prompt_sound=pin_prompt_sound,
            invalid_pin_sound=invalid_pin_sound,
            moh_sound=moh_sound,
            pin_prompt_tts=pin_prompt_tts,
            invalid_pin_tts=invalid_pin_tts,
            welcome_tts=welcome_tts,
            play_entry_tone=play_entry_tone,
            play_exit_tone=play_exit_tone,
            join_sound=join_sound,
            exit_sound=exit_sound,
            video_enabled=video_enabled,
            video_layout=video_layout,
            video_max_participants=video_max_participants,
            video_max_resolution=video_max_resolution,
            video_fps=video_fps,
            video_exclude_self=video_exclude_self,
            video_codecs=video_codecs,
            video_passthrough=video_passthrough,
            video_content_share=video_content_share,
            video_content_layout=video_content_layout,
            video_mode=video_mode,
            kind=kind,
            panelist_pin=panelist_pin,
            panelists=panelists,
            qa=qa,
            qa_anonymous=qa_anonymous,
            chat=chat,
        )


        create_meeting_request.additional_properties = d
        return create_meeting_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
