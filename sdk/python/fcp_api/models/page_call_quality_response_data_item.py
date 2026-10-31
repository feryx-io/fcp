from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageCallQualityResponseDataItem")



@_attrs_define
class PageCallQualityResponseDataItem:
    """ One phone's report of a call.

        Attributes:
            call_id (str):
            user_id (str):
            direction (str):
            codec (str):
            srtp (bool):
            packets_sent (int):
            packets_received (int):
            packets_expected (int):
            packets_lost (int):
            loss_pct (float): `packets_lost` of `packets_expected`, in percent.
            jitter_ms (float):
            mos (float):
            poor (bool): Either score under 3.1.
            reported_at (datetime.datetime):
            username (None | str | Unset): The reporting user's name, while they exist.
            device_id (None | str | Unset):
            duration_seconds (int | None | Unset):
            rtt_ms (float | None | Unset):
            far_loss_pct (float | None | Unset):
            far_jitter_ms (float | None | Unset):
            far_lost (int | None | Unset):
            far_mos (float | None | Unset):
            local_rtp (None | str | Unset):
            remote_rtp (None | str | Unset):
     """

    call_id: str
    user_id: str
    direction: str
    codec: str
    srtp: bool
    packets_sent: int
    packets_received: int
    packets_expected: int
    packets_lost: int
    loss_pct: float
    jitter_ms: float
    mos: float
    poor: bool
    reported_at: datetime.datetime
    username: None | str | Unset = UNSET
    device_id: None | str | Unset = UNSET
    duration_seconds: int | None | Unset = UNSET
    rtt_ms: float | None | Unset = UNSET
    far_loss_pct: float | None | Unset = UNSET
    far_jitter_ms: float | None | Unset = UNSET
    far_lost: int | None | Unset = UNSET
    far_mos: float | None | Unset = UNSET
    local_rtp: None | str | Unset = UNSET
    remote_rtp: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        user_id = self.user_id

        direction = self.direction

        codec = self.codec

        srtp = self.srtp

        packets_sent = self.packets_sent

        packets_received = self.packets_received

        packets_expected = self.packets_expected

        packets_lost = self.packets_lost

        loss_pct = self.loss_pct

        jitter_ms = self.jitter_ms

        mos = self.mos

        poor = self.poor

        reported_at = self.reported_at.isoformat()

        username: None | str | Unset
        if isinstance(self.username, Unset):
            username = UNSET
        else:
            username = self.username

        device_id: None | str | Unset
        if isinstance(self.device_id, Unset):
            device_id = UNSET
        else:
            device_id = self.device_id

        duration_seconds: int | None | Unset
        if isinstance(self.duration_seconds, Unset):
            duration_seconds = UNSET
        else:
            duration_seconds = self.duration_seconds

        rtt_ms: float | None | Unset
        if isinstance(self.rtt_ms, Unset):
            rtt_ms = UNSET
        else:
            rtt_ms = self.rtt_ms

        far_loss_pct: float | None | Unset
        if isinstance(self.far_loss_pct, Unset):
            far_loss_pct = UNSET
        else:
            far_loss_pct = self.far_loss_pct

        far_jitter_ms: float | None | Unset
        if isinstance(self.far_jitter_ms, Unset):
            far_jitter_ms = UNSET
        else:
            far_jitter_ms = self.far_jitter_ms

        far_lost: int | None | Unset
        if isinstance(self.far_lost, Unset):
            far_lost = UNSET
        else:
            far_lost = self.far_lost

        far_mos: float | None | Unset
        if isinstance(self.far_mos, Unset):
            far_mos = UNSET
        else:
            far_mos = self.far_mos

        local_rtp: None | str | Unset
        if isinstance(self.local_rtp, Unset):
            local_rtp = UNSET
        else:
            local_rtp = self.local_rtp

        remote_rtp: None | str | Unset
        if isinstance(self.remote_rtp, Unset):
            remote_rtp = UNSET
        else:
            remote_rtp = self.remote_rtp


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "user_id": user_id,
            "direction": direction,
            "codec": codec,
            "srtp": srtp,
            "packets_sent": packets_sent,
            "packets_received": packets_received,
            "packets_expected": packets_expected,
            "packets_lost": packets_lost,
            "loss_pct": loss_pct,
            "jitter_ms": jitter_ms,
            "mos": mos,
            "poor": poor,
            "reported_at": reported_at,
        })
        if username is not UNSET:
            field_dict["username"] = username
        if device_id is not UNSET:
            field_dict["device_id"] = device_id
        if duration_seconds is not UNSET:
            field_dict["duration_seconds"] = duration_seconds
        if rtt_ms is not UNSET:
            field_dict["rtt_ms"] = rtt_ms
        if far_loss_pct is not UNSET:
            field_dict["far_loss_pct"] = far_loss_pct
        if far_jitter_ms is not UNSET:
            field_dict["far_jitter_ms"] = far_jitter_ms
        if far_lost is not UNSET:
            field_dict["far_lost"] = far_lost
        if far_mos is not UNSET:
            field_dict["far_mos"] = far_mos
        if local_rtp is not UNSET:
            field_dict["local_rtp"] = local_rtp
        if remote_rtp is not UNSET:
            field_dict["remote_rtp"] = remote_rtp

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        user_id = d.pop("user_id")

        direction = d.pop("direction")

        codec = d.pop("codec")

        srtp = d.pop("srtp")

        packets_sent = d.pop("packets_sent")

        packets_received = d.pop("packets_received")

        packets_expected = d.pop("packets_expected")

        packets_lost = d.pop("packets_lost")

        loss_pct = d.pop("loss_pct")

        jitter_ms = d.pop("jitter_ms")

        mos = d.pop("mos")

        poor = d.pop("poor")

        reported_at = datetime.datetime.fromisoformat(d.pop("reported_at"))




        def _parse_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        username = _parse_username(d.pop("username", UNSET))


        def _parse_device_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        device_id = _parse_device_id(d.pop("device_id", UNSET))


        def _parse_duration_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_seconds = _parse_duration_seconds(d.pop("duration_seconds", UNSET))


        def _parse_rtt_ms(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        rtt_ms = _parse_rtt_ms(d.pop("rtt_ms", UNSET))


        def _parse_far_loss_pct(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_loss_pct = _parse_far_loss_pct(d.pop("far_loss_pct", UNSET))


        def _parse_far_jitter_ms(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_jitter_ms = _parse_far_jitter_ms(d.pop("far_jitter_ms", UNSET))


        def _parse_far_lost(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        far_lost = _parse_far_lost(d.pop("far_lost", UNSET))


        def _parse_far_mos(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_mos = _parse_far_mos(d.pop("far_mos", UNSET))


        def _parse_local_rtp(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        local_rtp = _parse_local_rtp(d.pop("local_rtp", UNSET))


        def _parse_remote_rtp(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        remote_rtp = _parse_remote_rtp(d.pop("remote_rtp", UNSET))


        page_call_quality_response_data_item = cls(
            call_id=call_id,
            user_id=user_id,
            direction=direction,
            codec=codec,
            srtp=srtp,
            packets_sent=packets_sent,
            packets_received=packets_received,
            packets_expected=packets_expected,
            packets_lost=packets_lost,
            loss_pct=loss_pct,
            jitter_ms=jitter_ms,
            mos=mos,
            poor=poor,
            reported_at=reported_at,
            username=username,
            device_id=device_id,
            duration_seconds=duration_seconds,
            rtt_ms=rtt_ms,
            far_loss_pct=far_loss_pct,
            far_jitter_ms=far_jitter_ms,
            far_lost=far_lost,
            far_mos=far_mos,
            local_rtp=local_rtp,
            remote_rtp=remote_rtp,
        )


        page_call_quality_response_data_item.additional_properties = d
        return page_call_quality_response_data_item

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
