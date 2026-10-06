from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageBrandAssetResponseDataItem")



@_attrs_define
class PageBrandAssetResponseDataItem:
    """ An uploaded file.

        Attributes:
            sha256 (str): Hex SHA-256: what the brand names it by.
            mime (str): What the file is, from its bytes.
            kind (str): `image` or `audio`.
            size (int):
            uploaded_at (datetime.datetime):
            width (int | None | Unset): A PNG's dimensions.
            height (int | None | Unset):
            file_name (None | str | Unset):
            url (None | str | Unset): Where apps fetch it, without credentials.
     """

    sha256: str
    mime: str
    kind: str
    size: int
    uploaded_at: datetime.datetime
    width: int | None | Unset = UNSET
    height: int | None | Unset = UNSET
    file_name: None | str | Unset = UNSET
    url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        sha256 = self.sha256

        mime = self.mime

        kind = self.kind

        size = self.size

        uploaded_at = self.uploaded_at.isoformat()

        width: int | None | Unset
        if isinstance(self.width, Unset):
            width = UNSET
        else:
            width = self.width

        height: int | None | Unset
        if isinstance(self.height, Unset):
            height = UNSET
        else:
            height = self.height

        file_name: None | str | Unset
        if isinstance(self.file_name, Unset):
            file_name = UNSET
        else:
            file_name = self.file_name

        url: None | str | Unset
        if isinstance(self.url, Unset):
            url = UNSET
        else:
            url = self.url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "sha256": sha256,
            "mime": mime,
            "kind": kind,
            "size": size,
            "uploaded_at": uploaded_at,
        })
        if width is not UNSET:
            field_dict["width"] = width
        if height is not UNSET:
            field_dict["height"] = height
        if file_name is not UNSET:
            field_dict["file_name"] = file_name
        if url is not UNSET:
            field_dict["url"] = url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        sha256 = d.pop("sha256")

        mime = d.pop("mime")

        kind = d.pop("kind")

        size = d.pop("size")

        uploaded_at = datetime.datetime.fromisoformat(d.pop("uploaded_at"))




        def _parse_width(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        width = _parse_width(d.pop("width", UNSET))


        def _parse_height(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        height = _parse_height(d.pop("height", UNSET))


        def _parse_file_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        file_name = _parse_file_name(d.pop("file_name", UNSET))


        def _parse_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        url = _parse_url(d.pop("url", UNSET))


        page_brand_asset_response_data_item = cls(
            sha256=sha256,
            mime=mime,
            kind=kind,
            size=size,
            uploaded_at=uploaded_at,
            width=width,
            height=height,
            file_name=file_name,
            url=url,
        )


        page_brand_asset_response_data_item.additional_properties = d
        return page_brand_asset_response_data_item

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
