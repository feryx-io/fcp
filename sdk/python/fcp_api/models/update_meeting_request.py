from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.video_codec import VideoCodec
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateMeetingRequest")



@_attrs_define
class UpdateMeetingRequest:
    """ Update meeting request body (all fields optional).

        Attributes:
            name (None | str | Unset):
            guest_pin (None | str | Unset):
            host_pin (None | str | Unset):
            require_guest_pin (bool | None | Unset):
            max_participants (int | None | Unset):
            wait_for_moderator (bool | None | Unset):
            enabled (bool | None | Unset):
            auto_record (bool | None | Unset):
            default_locked (bool | None | Unset):
            permanent (bool | None | Unset):
            owner_id (None | str | Unset):
            pin_prompt_sound (None | str | Unset):
            invalid_pin_sound (None | str | Unset):
            moh_sound (None | str | Unset):
            pin_prompt_tts (None | str | Unset):
            invalid_pin_tts (None | str | Unset):
            welcome_tts (None | str | Unset):
            play_entry_tone (bool | None | Unset):
            play_exit_tone (bool | None | Unset):
            join_sound (None | str | Unset):
            exit_sound (None | str | Unset):
            video_enabled (bool | None | Unset):
            video_layout (None | str | Unset):
            video_max_participants (int | None | Unset):
            video_max_resolution (None | str | Unset):
            video_fps (int | None | Unset):
            video_exclude_self (bool | None | Unset):
            video_codecs (list[VideoCodec] | None | Unset):
            video_passthrough (None | str | Unset):
            video_content_share (None | str | Unset):
            video_content_layout (None | str | Unset):
     """

    name: None | str | Unset = UNSET
    guest_pin: None | str | Unset = UNSET
    host_pin: None | str | Unset = UNSET
    require_guest_pin: bool | None | Unset = UNSET
    max_participants: int | None | Unset = UNSET
    wait_for_moderator: bool | None | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    auto_record: bool | None | Unset = UNSET
    default_locked: bool | None | Unset = UNSET
    permanent: bool | None | Unset = UNSET
    owner_id: None | str | Unset = UNSET
    pin_prompt_sound: None | str | Unset = UNSET
    invalid_pin_sound: None | str | Unset = UNSET
    moh_sound: None | str | Unset = UNSET
    pin_prompt_tts: None | str | Unset = UNSET
    invalid_pin_tts: None | str | Unset = UNSET
    welcome_tts: None | str | Unset = UNSET
    play_entry_tone: bool | None | Unset = UNSET
    play_exit_tone: bool | None | Unset = UNSET
    join_sound: None | str | Unset = UNSET
    exit_sound: None | str | Unset = UNSET
    video_enabled: bool | None | Unset = UNSET
    video_layout: None | str | Unset = UNSET
    video_max_participants: int | None | Unset = UNSET
    video_max_resolution: None | str | Unset = UNSET
    video_fps: int | None | Unset = UNSET
    video_exclude_self: bool | None | Unset = UNSET
    video_codecs: list[VideoCodec] | None | Unset = UNSET
    video_passthrough: None | str | Unset = UNSET
    video_content_share: None | str | Unset = UNSET
    video_content_layout: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
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

        require_guest_pin: bool | None | Unset
        if isinstance(self.require_guest_pin, Unset):
            require_guest_pin = UNSET
        else:
            require_guest_pin = self.require_guest_pin

        max_participants: int | None | Unset
        if isinstance(self.max_participants, Unset):
            max_participants = UNSET
        else:
            max_participants = self.max_participants

        wait_for_moderator: bool | None | Unset
        if isinstance(self.wait_for_moderator, Unset):
            wait_for_moderator = UNSET
        else:
            wait_for_moderator = self.wait_for_moderator

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        auto_record: bool | None | Unset
        if isinstance(self.auto_record, Unset):
            auto_record = UNSET
        else:
            auto_record = self.auto_record

        default_locked: bool | None | Unset
        if isinstance(self.default_locked, Unset):
            default_locked = UNSET
        else:
            default_locked = self.default_locked

        permanent: bool | None | Unset
        if isinstance(self.permanent, Unset):
            permanent = UNSET
        else:
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

        play_entry_tone: bool | None | Unset
        if isinstance(self.play_entry_tone, Unset):
            play_entry_tone = UNSET
        else:
            play_entry_tone = self.play_entry_tone

        play_exit_tone: bool | None | Unset
        if isinstance(self.play_exit_tone, Unset):
            play_exit_tone = UNSET
        else:
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

        video_enabled: bool | None | Unset
        if isinstance(self.video_enabled, Unset):
            video_enabled = UNSET
        else:
            video_enabled = self.video_enabled

        video_layout: None | str | Unset
        if isinstance(self.video_layout, Unset):
            video_layout = UNSET
        else:
            video_layout = self.video_layout

        video_max_participants: int | None | Unset
        if isinstance(self.video_max_participants, Unset):
            video_max_participants = UNSET
        else:
            video_max_participants = self.video_max_participants

        video_max_resolution: None | str | Unset
        if isinstance(self.video_max_resolution, Unset):
            video_max_resolution = UNSET
        else:
            video_max_resolution = self.video_max_resolution

        video_fps: int | None | Unset
        if isinstance(self.video_fps, Unset):
            video_fps = UNSET
        else:
            video_fps = self.video_fps

        video_exclude_self: bool | None | Unset
        if isinstance(self.video_exclude_self, Unset):
            video_exclude_self = UNSET
        else:
            video_exclude_self = self.video_exclude_self

        video_codecs: list[str] | None | Unset
        if isinstance(self.video_codecs, Unset):
            video_codecs = UNSET
        elif isinstance(self.video_codecs, list):
            video_codecs = []
            for video_codecs_type_0_item_data in self.video_codecs:
                video_codecs_type_0_item = video_codecs_type_0_item_data.value
                video_codecs.append(video_codecs_type_0_item)


        else:
            video_codecs = self.video_codecs

        video_passthrough: None | str | Unset
        if isinstance(self.video_passthrough, Unset):
            video_passthrough = UNSET
        else:
            video_passthrough = self.video_passthrough

        video_content_share: None | str | Unset
        if isinstance(self.video_content_share, Unset):
            video_content_share = UNSET
        else:
            video_content_share = self.video_content_share

        video_content_layout: None | str | Unset
        if isinstance(self.video_content_layout, Unset):
            video_content_layout = UNSET
        else:
            video_content_layout = self.video_content_layout


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if name is not UNSET:
            field_dict["name"] = name
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

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


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


        def _parse_require_guest_pin(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        require_guest_pin = _parse_require_guest_pin(d.pop("require_guest_pin", UNSET))


        def _parse_max_participants(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_participants = _parse_max_participants(d.pop("max_participants", UNSET))


        def _parse_wait_for_moderator(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        wait_for_moderator = _parse_wait_for_moderator(d.pop("wait_for_moderator", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_auto_record(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        auto_record = _parse_auto_record(d.pop("auto_record", UNSET))


        def _parse_default_locked(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        default_locked = _parse_default_locked(d.pop("default_locked", UNSET))


        def _parse_permanent(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        permanent = _parse_permanent(d.pop("permanent", UNSET))


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


        def _parse_play_entry_tone(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        play_entry_tone = _parse_play_entry_tone(d.pop("play_entry_tone", UNSET))


        def _parse_play_exit_tone(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        play_exit_tone = _parse_play_exit_tone(d.pop("play_exit_tone", UNSET))


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


        def _parse_video_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        video_enabled = _parse_video_enabled(d.pop("video_enabled", UNSET))


        def _parse_video_layout(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        video_layout = _parse_video_layout(d.pop("video_layout", UNSET))


        def _parse_video_max_participants(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        video_max_participants = _parse_video_max_participants(d.pop("video_max_participants", UNSET))


        def _parse_video_max_resolution(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        video_max_resolution = _parse_video_max_resolution(d.pop("video_max_resolution", UNSET))


        def _parse_video_fps(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        video_fps = _parse_video_fps(d.pop("video_fps", UNSET))


        def _parse_video_exclude_self(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        video_exclude_self = _parse_video_exclude_self(d.pop("video_exclude_self", UNSET))


        def _parse_video_codecs(data: object) -> list[VideoCodec] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                video_codecs_type_0 = []
                _video_codecs_type_0 = data
                for video_codecs_type_0_item_data in (_video_codecs_type_0):
                    video_codecs_type_0_item = VideoCodec(video_codecs_type_0_item_data)



                    video_codecs_type_0.append(video_codecs_type_0_item)

                return video_codecs_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[VideoCodec] | None | Unset, data)

        video_codecs = _parse_video_codecs(d.pop("video_codecs", UNSET))


        def _parse_video_passthrough(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        video_passthrough = _parse_video_passthrough(d.pop("video_passthrough", UNSET))


        def _parse_video_content_share(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        video_content_share = _parse_video_content_share(d.pop("video_content_share", UNSET))


        def _parse_video_content_layout(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        video_content_layout = _parse_video_content_layout(d.pop("video_content_layout", UNSET))


        update_meeting_request = cls(
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
        )


        update_meeting_request.additional_properties = d
        return update_meeting_request

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
