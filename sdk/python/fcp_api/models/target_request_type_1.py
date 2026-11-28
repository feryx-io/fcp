from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.target_request_type_1_type import TargetRequestType1Type
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TargetRequestType1")



@_attrs_define
class TargetRequestType1:
    """ 
        Attributes:
            endpoint (str):
            bucket (str):
            access_key_id (str):
            type_ (TargetRequestType1Type):
            region (None | str | Unset):
            prefix (None | str | Unset):
            secret_access_key (None | str | Unset): Write-only.
     """

    endpoint: str
    bucket: str
    access_key_id: str
    type_: TargetRequestType1Type
    region: None | str | Unset = UNSET
    prefix: None | str | Unset = UNSET
    secret_access_key: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        endpoint = self.endpoint

        bucket = self.bucket

        access_key_id = self.access_key_id

        type_ = self.type_.value

        region: None | str | Unset
        if isinstance(self.region, Unset):
            region = UNSET
        else:
            region = self.region

        prefix: None | str | Unset
        if isinstance(self.prefix, Unset):
            prefix = UNSET
        else:
            prefix = self.prefix

        secret_access_key: None | str | Unset
        if isinstance(self.secret_access_key, Unset):
            secret_access_key = UNSET
        else:
            secret_access_key = self.secret_access_key


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "endpoint": endpoint,
            "bucket": bucket,
            "access_key_id": access_key_id,
            "type": type_,
        })
        if region is not UNSET:
            field_dict["region"] = region
        if prefix is not UNSET:
            field_dict["prefix"] = prefix
        if secret_access_key is not UNSET:
            field_dict["secret_access_key"] = secret_access_key

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        endpoint = d.pop("endpoint")

        bucket = d.pop("bucket")

        access_key_id = d.pop("access_key_id")

        type_ = TargetRequestType1Type(d.pop("type"))




        def _parse_region(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        region = _parse_region(d.pop("region", UNSET))


        def _parse_prefix(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        prefix = _parse_prefix(d.pop("prefix", UNSET))


        def _parse_secret_access_key(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        secret_access_key = _parse_secret_access_key(d.pop("secret_access_key", UNSET))


        target_request_type_1 = cls(
            endpoint=endpoint,
            bucket=bucket,
            access_key_id=access_key_id,
            type_=type_,
            region=region,
            prefix=prefix,
            secret_access_key=secret_access_key,
        )


        target_request_type_1.additional_properties = d
        return target_request_type_1

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
